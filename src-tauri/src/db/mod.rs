mod models;
mod ops;
mod schema;
use crate::db::models::*;
use core::fmt;
use diesel::{
    backend::Backend,
    connection::SimpleConnection,
    deserialize::FromSql,
    prelude::*,
    serialize::{self, IsNull, Output, ToSql},
    sql_types::{self, Integer},
};
use pgn_reader::{BufferedReader, Color, Outcome, RawHeader, SanPlus, Skip, Visitor};
use serde::{Deserialize, Serialize};
use serde_with::{formats::SpaceSeparator, serde_as, DisplayFromStr, StringWithSeparator};
use std::{
    ffi::OsStr,
    fs::File,
    io, mem,
    path::{Path, PathBuf},
    str::FromStr,
};
use tauri::{
    api::path::{resolve_path, BaseDirectory},
    Manager,
};

use self::{
    ops::{create_game, create_player, increment_game_count},
    schema::{
        games::{self, BoxedQuery},
        players,
    },
};

#[derive(
    Debug,
    Copy,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    diesel::AsExpression,
    diesel::FromSqlRow,
)]
#[diesel(sql_type = sql_types::Integer)]
pub enum Speed {
    UltraBullet,
    Bullet,
    Blitz,
    Rapid,
    Classical,
    Correspondence,
}

impl<DB> FromSql<sql_types::Integer, DB> for Speed
where
    DB: Backend,
    i32: FromSql<sql_types::Integer, DB>,
{
    fn from_nullable_sql(
        bytes: Option<diesel::backend::RawValue<'_, DB>>,
    ) -> diesel::deserialize::Result<Self> {
        let u = i32::from_nullable_sql(bytes)?;
        Ok(Speed::from(u as u8))
    }

    fn from_sql(bytes: diesel::backend::RawValue<'_, DB>) -> diesel::deserialize::Result<Self> {
        let u = i32::from_sql(bytes)?;
        Ok(Speed::from(u as u8))
    }
}

impl ToSql<Integer, diesel::sqlite::Sqlite> for Speed
where
    i32: ToSql<Integer, diesel::sqlite::Sqlite>,
{
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, diesel::sqlite::Sqlite>) -> serialize::Result {
        out.set_value(*self as i32);
        Ok(IsNull::No)
    }
}

impl Speed {
    fn from_seconds_and_increment(seconds: u64, increment: u64) -> Speed {
        let total = seconds + 40 * increment;

        if total < 30 {
            Speed::UltraBullet
        } else if total < 180 {
            Speed::Bullet
        } else if total < 480 {
            Speed::Blitz
        } else if total < 1500 {
            Speed::Rapid
        } else if total < 21_600 {
            Speed::Classical
        } else {
            Speed::Correspondence
        }
    }

    fn from_bytes(bytes: &[u8]) -> Result<Speed, ()> {
        if bytes == b"-" {
            return Ok(Speed::Correspondence);
        }

        let mut parts = bytes.splitn(2, |ch| *ch == b'+');
        let seconds = btoi::btou(parts.next().ok_or(())?).map_err(|_| ())?;
        let increment = btoi::btou(parts.next().ok_or(())?).map_err(|_| ())?;
        Ok(Speed::from_seconds_and_increment(seconds, increment))
    }

    fn from(u: u8) -> Speed {
        match u {
            0 => Speed::UltraBullet,
            1 => Speed::Bullet,
            2 => Speed::Blitz,
            3 => Speed::Rapid,
            4 => Speed::Classical,
            5 => Speed::Correspondence,
            _ => unreachable!(),
        }
    }
}

struct Batch {
    games: Vec<TempGame>,
}

#[derive(Default, Debug, Serialize)]
pub struct TempPlayer {
    id: usize,
    name: Option<String>,
    rating: Option<i32>,
}

#[serde_as]
#[derive(Default, Debug, Serialize)]
struct TempGame {
    speed: Option<Speed>,
    fen: Option<String>,
    site: Option<String>,
    date: Option<String>,
    white: TempPlayer,
    black: TempPlayer,
    #[serde_as(as = "Option<DisplayFromStr>")]
    outcome: Option<Outcome>,
    #[serde_as(as = "StringWithSeparator<SpaceSeparator, SanPlus>")]
    moves: Vec<SanPlus>,
}

struct Importer {
    db: diesel::SqliteConnection,
    batch_size: usize,
    current: TempGame,
    skip: bool,
    batch: Vec<TempGame>,
}

impl Importer {
    fn new(batch_size: usize, db: diesel::SqliteConnection) -> Importer {
        Importer {
            db,
            batch_size,
            current: TempGame::default(),
            skip: false,
            batch: Vec::with_capacity(batch_size),
        }
    }

    pub fn send(&mut self) -> Result<(), diesel::result::Error> {
        let batch = Batch {
            games: mem::replace(&mut self.batch, Vec::with_capacity(self.batch_size)),
        };

        for game in batch.games {
            let white;
            let black;
            if let Some(name) = game.white.name {
                white = create_player(&mut self.db, &name)?;
            } else {
                white = Player::default();
            }
            if let Some(name) = game.black.name {
                black = create_player(&mut self.db, &name)?;
            } else {
                black = Player::default();
            }

            let moves: Vec<String> = game.moves.iter().map(|m| m.to_string()).collect();

            let new_game = NewGame {
                white: white.id,
                black: black.id,
                white_rating: game.white.rating,
                black_rating: game.black.rating,
                date: game.date.as_deref(),
                speed: game.speed,
                site: game.site.as_deref(),
                fen: game.fen.as_deref(),
                outcome: game.outcome.map(|r| match r {
                    Outcome::Decisive { winner } => match winner {
                        Color::White => 1,
                        Color::Black => 2,
                    },
                    Outcome::Draw => 3,
                }),
                moves: &moves.join(" "),
            };

            create_game(&mut self.db, new_game).map_err(|e| {
                println!("Error: {:?}", e);
                e
            })?;
            increment_game_count(&mut self.db, white.id);
            increment_game_count(&mut self.db, black.id);
        }
        Ok(())
    }
}

impl Visitor for Importer {
    type Result = ();

    fn begin_game(&mut self) {
        self.skip = false;
        self.current = TempGame::default();
    }

    fn header(&mut self, key: &[u8], value: RawHeader<'_>) {
        if key == b"White" {
            self.current.white.name = Some(value.decode_utf8().expect("White").into_owned());
        } else if key == b"Black" {
            self.current.black.name = Some(value.decode_utf8().expect("Black").into_owned());
        } else if key == b"WhiteElo" {
            if value.as_bytes() != b"?" {
                self.current.white.rating = Some(btoi::btoi(value.as_bytes()).expect("WhiteElo"));
            }
        } else if key == b"BlackElo" {
            if value.as_bytes() != b"?" {
                self.current.black.rating = Some(btoi::btoi(value.as_bytes()).expect("BlackElo"));
            }
        } else if key == b"TimeControl" {
            self.current.speed = Some(Speed::from_bytes(value.as_bytes()).expect("TimeControl"));
        } else if key == b"Date" || key == b"UTCDate" {
            self.current.date = Some(String::from_utf8(value.as_bytes().to_owned()).expect("Date"));
        } else if key == b"WhiteTitle" || key == b"BlackTitle" {
            if value.as_bytes() == b"BOT" {
                self.skip = true;
            }
        } else if key == b"Site" {
            self.current.site = Some(String::from_utf8(value.as_bytes().to_owned()).expect("Site"));
        } else if key == b"Result" {
            match Outcome::from_ascii(value.as_bytes()) {
                Ok(outcome) => self.current.outcome = Some(outcome),
                Err(_) => self.skip = true,
            }
        } else if key == b"FEN" {
            if value.as_bytes() == b"rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1" {
                self.current.fen = None;
            } else {
                self.current.fen = Some(value.decode_utf8().expect("FEN").into_owned());
            }
        }
    }

    fn end_headers(&mut self) -> Skip {
        self.skip |= self.current.white.rating.is_none() || self.current.black.rating.is_none();
        Skip(self.skip)
    }

    fn san(&mut self, san: SanPlus) {
        self.current.moves.push(san);
    }

    fn begin_variation(&mut self) -> Skip {
        Skip(true) // stay in the mainline
    }

    fn end_game(&mut self) {
        if !self.skip {
            self.batch.push(mem::take(&mut self.current));
        }

        if self.batch.len() >= self.batch_size {
            self.send();
        }
    }
}

#[tauri::command]
pub async fn convert_pgn(file: PathBuf, app: tauri::AppHandle) -> Result<(), String> {
    // get the name of the file without the extension
    let filename = file.file_stem().expect("file name");
    let extension = file.extension().expect("file extension");
    let db_filename = Path::new("db").join(filename).with_extension("sqlite");

    // export the database to the AppData folder
    let destination = resolve_path(
        &app.config(),
        app.package_info(),
        &app.env(),
        &db_filename,
        Some(BaseDirectory::AppData),
    )
    .expect("resolve path");

    // create the database file
    let mut db = SqliteConnection::establish(destination.to_str().unwrap())
        .or(Err("Failed to create database file"))?;

    // add pragmas to be more performant
    db.batch_execute(
        "PRAGMA journal_mode = OFF;
        PRAGMA synchronous = 0;
        PRAGMA locking_mode = EXCLUSIVE;
        PRAGMA temp_store = MEMORY;",
    )
    .or(Err("Failed to add pragmas"))?;

    // create the players table if it doesn't exist

    db.batch_execute(
        "CREATE TABLE IF NOT EXISTS players (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            game_count INTEGER DEFAULT 0
        )",
    )
    .expect("create players table");

    // create the games table if it doesn't exist
    db.batch_execute(
        "CREATE TABLE IF NOT EXISTS games (
                    id INTEGER PRIMARY KEY,
                    white INTEGER NOT NULL,
                    black INTEGER NOT NULL,
                    white_rating INTEGER,
                    black_rating INTEGER,
                    date TEXT NOT NULL,
                    speed INTEGER,
                    site TEXT,
                    fen TEXT,
                    outcome INTEGER NOT NULL,
                    moves TEXT NOT NULL,
                    FOREIGN KEY(white) REFERENCES players(id),
                    FOREIGN KEY(black) REFERENCES players(id)
        )",
    )
    .expect("create games table");

    // create the metadata table
    db.batch_execute(
        "CREATE TABLE IF NOT EXISTS metadata (
                    key TEXT NOT NULL,
                    value TEXT NOT NULL
        )",
    )
    .expect("create metadata table");

    // add an untitled title to the metadata table
    db.batch_execute("INSERT OR IGNORE INTO metadata (key, value) VALUES ('title', 'Untitled')")
        .expect("insert title");

    let file = File::open(&file).expect("open pgn file");

    let uncompressed: Box<dyn io::Read> = if extension == OsStr::new("bz2") {
        Box::new(bzip2::read::MultiBzDecoder::new(file))
    } else if extension == OsStr::new("zst") {
        Box::new(zstd::Decoder::new(file).expect("zstd decoder"))
    } else {
        Box::new(file)
    };

    let mut reader = BufferedReader::new(uncompressed);
    let mut importer = Importer::new(50, db);
    reader.read_all(&mut importer).expect("read pgn file");
    importer.send().map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize)]
pub struct DatabaseInfo {
    title: String,
    description: String,
    player_count: usize,
    game_count: usize,
    storage_size: usize,
}

#[tauri::command]
pub async fn get_db_info(file: PathBuf, app: tauri::AppHandle) -> Result<DatabaseInfo, String> {
    let db_path = PathBuf::from("db").join(file);

    let path = resolve_path(
        &app.config(),
        app.package_info(),
        &app.env(),
        &db_path,
        Some(BaseDirectory::AppData),
    )
    .or(Err("resolve path"))?;

    let db = rusqlite::Connection::open(&path).expect("open database");
    let mut stmt = db
        .prepare("SELECT COUNT(*) FROM players")
        .expect("prepare player count");
    let player_count = stmt
        .query_row([], |row| row.get(0))
        .expect("get player count");

    let mut stmt = db
        .prepare("SELECT COUNT(*) FROM games")
        .expect("prepare game count");
    let game_count = stmt
        .query_row([], |row| row.get(0))
        .expect("get game count");

    // get the title from the metadata table
    let mut stmt = db
        .prepare("SELECT value FROM metadata WHERE key = 'title'")
        .expect("prepare title");
    let title = stmt.query_row([], |row| row.get(0)).expect("get title");

    let storage_size = path.metadata().expect("get metadata").len() as usize;
    let filename = path.file_name().expect("get filename").to_string_lossy();

    Ok(DatabaseInfo {
        title,
        description: filename.to_string(),
        player_count,
        game_count,
        storage_size,
    })
}

#[tauri::command]
pub async fn rename_db(file: PathBuf, title: String) -> Result<(), String> {
    let db = rusqlite::Connection::open(file).expect("open database");
    db.execute("UPDATE metadata SET value = ? WHERE key = 'title'", [title])
        .expect("update title");
    Ok(())
}

#[tauri::command]
pub async fn get_number_games(file: PathBuf) -> u64 {
    let db = rusqlite::Connection::open(file).expect("open database");
    db.query_row("SELECT COUNT(*) FROM game", [], |row| row.get(0))
        .expect("count games")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Sides {
    BlackWhite,
    WhiteBlack,
    Any,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Sort {
    Date,
    Rating,
    Speed,
    Outcome,
}

impl FromStr for Sort {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "date" => Ok(Sort::Date),
            "rating" => Ok(Sort::Rating),
            "speed" => Ok(Sort::Speed),
            "outcome" => Ok(Sort::Outcome),
            _ => Err(format!("invalid sort: {}", s)),
        }
    }
}

#[serde_as]
#[derive(Debug, Clone, Deserialize)]
pub struct GameQuery {
    pub skip_count: bool,
    pub player1: Option<String>,
    pub player2: Option<String>,
    pub range1: Option<(u16, u16)>,
    pub range2: Option<(u16, u16)>,
    pub sides: Option<Sides>,
    pub speed: Option<Speed>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub outcome: Option<Outcome>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub sort: Option<Sort>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryResponse<T> {
    pub data: T,
    pub count: Option<i64>,
}

#[tauri::command]
pub async fn get_games(
    file: PathBuf,
    query: GameQuery,
) -> QueryResponse<Vec<(Game, Player, Player)>> {
    let mut db =
        diesel::SqliteConnection::establish(&file.to_str().unwrap()).expect("open database");

    let mut count: Option<i64> = None;

    let (white_players, black_players) = diesel::alias!(players as white, players as black);
    let mut sql_query = games::table
        .inner_join(white_players.on(games::white.eq(white_players.field(players::id))))
        .inner_join(black_players.on(games::black.eq(black_players.field(players::id))))
        .into_boxed();
    let mut count_query = games::table
        .inner_join(white_players.on(games::white.eq(white_players.field(players::id))))
        .inner_join(black_players.on(games::black.eq(black_players.field(players::id))))
        .into_boxed();

    if let Some(player1) = query.player1 {
        sql_query = sql_query.filter(white_players.field(players::name).eq(player1.clone()));
        count_query = count_query.filter(white_players.field(players::name).eq(player1));
    }

    if let Some(speed) = query.speed {
        sql_query = sql_query.filter(games::speed.eq(speed as i32));
        count_query = count_query.filter(games::speed.eq(speed as i32));
    }

    if let Some(outcome) = query.outcome {
        sql_query = sql_query.filter(games::outcome.eq(match outcome {
            Outcome::Decisive { winner } => match winner {
                Color::White => 1,
                Color::Black => 2,
            },
            Outcome::Draw => 3,
        }));
        count_query = count_query.filter(games::outcome.eq(match outcome {
            Outcome::Decisive { winner } => match winner {
                Color::White => 1,
                Color::Black => 2,
            },
            Outcome::Draw => 3,
        }));
    }

    if !query.skip_count {
        count = Some(
            count_query
                .select(diesel::dsl::count(games::id))
                .first(&mut db)
                .expect("count games"),
        );
    }

    if let Some(limit) = query.limit {
        sql_query = sql_query.limit(limit);
    }

    if let Some(offset) = query.offset {
        sql_query = sql_query.offset(offset);
    }

    if let Some(sort) = query.sort {
        sql_query = match sort {
            Sort::Date => sql_query.order(games::date.desc()),
            Sort::Rating => {
                // sort by the greatest rating between game::white_rating and game::black_rating
                sql_query.order(diesel::dsl::sql::<diesel::sql_types::Integer>(
                    "MAX(games.white_rating, games.black_rating) DESC",
                ))
            }
            Sort::Speed => sql_query.order(games::speed.desc()),
            Sort::Outcome => sql_query.order(games::outcome.desc()),
        };
    }
    let games = sql_query.load(&mut db).expect("load games");

    QueryResponse { data: games, count }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlayerQuery {
    pub skip_count: bool,
    pub name: Option<String>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

#[tauri::command]
pub async fn get_players(file: PathBuf, query: PlayerQuery) -> QueryResponse<Vec<Player>> {
    let mut db =
        diesel::SqliteConnection::establish(&file.to_str().unwrap()).expect("open database");

    let mut players = Vec::new();
    let mut count = None;

    let mut sql_query = players::table.into_boxed();
    let mut count_query = players::table.into_boxed();

    if let Some(name) = query.name {
        sql_query = sql_query.filter(players::name.like(format!("%{}%", name)));
        count_query = count_query.filter(players::name.like(format!("%{}%", name)));
    }

    if !query.skip_count {
        count = Some(
            count_query
                .count()
                .get_result(&mut db)
                .expect("count players"),
        );
    }

    if let Some(limit) = query.limit {
        sql_query = sql_query.limit(limit as i64);
    }

    if let Some(offset) = query.offset {
        sql_query = sql_query.offset(offset as i64);
    }

    for player in sql_query.load::<Player>(&mut db).expect("load players") {
        players.push(player);
    }

    QueryResponse {
        data: players,
        count,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PlayerGameInfo {
    pub won: usize,
    pub lost: usize,
    pub draw: usize,
}

#[tauri::command]
pub async fn get_players_game_info(file: PathBuf, id: i32) -> PlayerGameInfo {
    let mut db =
        diesel::SqliteConnection::establish(&file.to_str().unwrap()).expect("open database");

    let mut info = PlayerGameInfo {
        won: 0,
        lost: 0,
        draw: 0,
    };

    let games = games::table
        .filter(games::white.eq(id).or(games::black.eq(id)))
        .load::<Game>(&mut db)
        .expect("load games");

    for game in games {
        match game.outcome {
            Some(1) => info.won += 1,
            Some(2) => info.lost += 1,
            Some(3) => info.draw += 1,
            _ => {}
        }
    }

    info
}
