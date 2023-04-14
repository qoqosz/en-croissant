import {
  Accordion,
  Button,
  Flex,
  Group,
  ScrollArea,
  Stack,
  Tabs,
} from "@mantine/core";
import {
  useForceUpdate,
  useHotkeys,
  useSessionStorage,
  useToggle,
  useViewportSize,
} from "@mantine/hooks";
import {
  IconDatabase,
  IconInfoCircle,
  IconNotes,
  IconRobot,
  IconZoomCheck,
} from "@tabler/icons-react";
import { save } from "@tauri-apps/api/dialog";
import { writeTextFile } from "@tauri-apps/api/fs";
import { Chess, Color, PieceSymbol, Square } from "chess.js";
import { useEffect, useMemo, useState } from "react";
import { VariationTree, goToPosition, parsePGN } from "../../utils/chess";
import { CompleteGame, defaultGame } from "../../utils/db";
import { Engine } from "../../utils/engines";
import { invoke, useLocalFile } from "../../utils/misc";
import { Tab } from "../../utils/tabs";
import GameInfo from "../common/GameInfo";
import MoveControls from "../common/MoveControls";
import TreeContext from "../common/TreeContext";
import BestMoves from "../panels/analysis/BestMoves";
import EngineSettingsBoard from "../panels/analysis/EngineSettingsBoard";
import ReportModal from "../panels/analysis/ReportModal";
import AnnotationPanel from "../panels/annotation/AnnotationPanel";
import DatabasePanel from "../panels/database/DatabasePanel";
import FenInput from "../panels/info/FenInput";
import PgnInput from "../panels/info/PgnInput";
import BoardPlay from "./BoardPlay";
import GameNotation from "./GameNotation";

function BoardAnalysis({
  id,
  tabs,
  setTabs,
}: {
  id: string;
  tabs: Tab[];
  setTabs: React.Dispatch<React.SetStateAction<Tab[]>>;
}) {
  const [completeGame, setCompleteGame] = useSessionStorage<CompleteGame>({
    key: id,
    defaultValue: { game: defaultGame(), currentMove: [] },
  });
  const game = completeGame.game;

  const forceUpdate = useForceUpdate();
  const [editingMode, toggleEditingMode] = useToggle();
  const [reportingMode, toggleReportingMode] = useToggle();
  const [analysisLoading, setAnalysisLoading] = useState(false);
  const [engines, setEngines] = useLocalFile<Engine[]>(
    "engines/engines.json",
    []
  );

  const initial_tree = useMemo(() => {
    const storedTree = sessionStorage.getItem(id + "-tree");
    if (storedTree) {
      const { pgn, currentMove } = JSON.parse(storedTree);
      const tree = parsePGN(pgn);
      if (tree.children.length > 0) {
        const treeAtPosition = goToPosition(tree, currentMove);
        return treeAtPosition;
      }
    }
    const tree = parsePGN(game.moves);
    return tree;
  }, [game.moves]);

  // Variation tree of all the previous moves
  const [tree, setTree] = useSessionStorage<VariationTree>({
    key: id + "-tree",
    defaultValue: initial_tree,
    serialize: (value) => {
      const storedTree = JSON.stringify({
        pgn: value.getTopVariation().getPGN({ headers: game }),
        currentMove: value.getPosition(),
      });
      return storedTree;
    },
    deserialize: (value) => {
      const { pgn, currentMove } = JSON.parse(value);
      const tree = parsePGN(pgn);
      const treeAtPosition = goToPosition(tree, currentMove);
      return treeAtPosition;
    },
  });
  useEffect(() => {
    setTree(initial_tree);
  }, [initial_tree]);
  const [arrows, setArrows] = useState<string[]>([]);
  let chess: Chess | null;
  try {
    chess = new Chess(tree.fen);
  } catch (e) {
    chess = null;
  }
  let turn = chess?.turn();

  function makeMove(move: { from: Square; to: Square; promotion?: string }) {
    if (chess === null) {
      invoke("make_move", {
        fen: tree.fen,
        from: move.from,
        to: move.to,
      }).then((fen) => {
        const newTree = new VariationTree(null, fen as string, null);
        setTree(newTree);
      });
    } else if (editingMode) {
      const piece = chess.get(move.from);
      chess.remove(move.to);
      chess.remove(move.from);
      chess.put(piece, move.to);
      const newTree = new VariationTree(null, chess.fen(), null);
      setTree(newTree);
    } else {
      const newMove = chess.move(move);
      const newTree = new VariationTree(tree, chess.fen(), newMove);
      if (tree.children.length === 0) {
        tree.children = [newTree];
        setTree(newTree);
      } else if (tree.children.every((child) => child.fen !== chess!.fen())) {
        tree.children.push(newTree);
        setTree(newTree);
      } else {
        const child = tree.children.find((child) => child.fen === chess!.fen());
        setTree(child!);
      }
    }
  }

  function makeMoves(moves: string[]) {
    let parentTree = tree;
    let newTree = tree;
    moves.forEach((move) => {
      const newMove = chess!.move(move);
      newTree = new VariationTree(parentTree, chess!.fen(), newMove);
      if (parentTree.children.length === 0) {
        parentTree.children = [newTree];
        parentTree = newTree;
      } else if (
        parentTree.children.every((child) => child.fen !== newTree.fen)
      ) {
        parentTree.children.push(newTree);
        parentTree = newTree;
      } else {
        parentTree = parentTree.children.find(
          (child) => child.fen === newTree.fen
        )!;
      }
    });
    setTree(newTree);
  }

  function deleteVariation() {
    if (tree.parent) {
      tree.parent.children = tree.parent.children.filter(
        (child) => !child.equals(tree)
      );
      setTree(tree.parent);
    }
  }

  function promoteVariation() {
    if (tree.parent) {
      const parent = tree.parent;
      parent.children = [
        tree,
        ...parent.children.filter((child) => !child.equals(tree)),
      ];
      forceUpdate();
    }
  }

  function addPiece(square: Square, piece: PieceSymbol, color: Color) {
    let newTree: VariationTree;
    if (chess) {
      chess.put({ type: piece, color }, square);
      newTree = new VariationTree(null, chess.fen(), null);
      setTree(newTree);
    } else {
      invoke("put_piece", {
        fen: tree.fen,
        square,
        piece,
        color,
      }).then((fen) => {
        newTree = new VariationTree(null, fen as string, null);
        setTree(newTree);
      });
    }
  }

  function undoMove() {
    if (tree.parent) {
      setTree(tree.parent);
    }
  }

  function redoMove() {
    if (tree.children.length > 0) {
      setTree(tree.children[0]);
    }
  }

  function goToStart() {
    setTree(tree.getTopVariation());
  }

  function goToEnd() {
    setTree(tree.getBottomVariation());
  }

  function resetToFen(fen: string) {
    setTree(new VariationTree(null, fen, null));
  }

  function changeToPlayMode() {
    setTabs(
      tabs.map((tab) => (tab.value === id ? { ...tab, type: "play" } : tab))
    );
  }

  async function saveFile() {
    const filePath = await save({
      filters: [
        {
          name: "PGN",
          extensions: ["pgn"],
        },
      ],
    });
    if (filePath)
      await writeTextFile(
        filePath,
        tree.getTopVariation().getPGN({ headers: game })
      );
  }

  useHotkeys([
    ["ArrowLeft", () => undoMove()],
    ["ArrowRight", () => redoMove()],
    ["ArrowUp", () => goToStart()],
    ["ArrowDown", () => goToEnd()],
    ["Delete", () => deleteVariation()],
    ["Ctrl+S", () => saveFile()],
  ]);

  useEffect(() => {
    setArrows([]);
  }, [tree.fen]);

  const { height, width } = useViewportSize();

  function getBoardSize(height: number, width: number) {
    const initial = Math.min((height - 140) * 0.95, width * 0.4);
    if (width < 680) {
      return width - 120;
    }
    return initial;
  }
  const boardSize = getBoardSize(height, width);

  return (
    <TreeContext.Provider value={tree}>
      <ReportModal
        moves={tree.getTopVariation().getPGN({
          headers: game,
          comments: false,
          specialSymbols: false,
          symbols: false,
        })}
        reportingMode={reportingMode}
        toggleReportingMode={toggleReportingMode}
        setLoading={setAnalysisLoading}
        setTree={setTree}
      />
      <Flex gap="md" wrap="wrap" align="start">
        <BoardPlay
          makeMove={makeMove}
          arrows={arrows}
          forceUpdate={forceUpdate}
          setTree={setTree}
          editingMode={editingMode}
          toggleEditingMode={toggleEditingMode}
          setCompleteGame={setCompleteGame}
          completeGame={completeGame}
          addPiece={addPiece}
        />
        <Stack
          sx={{
            flex: 1,
            flexGrow: 1,
            justifyContent: "space-between",
            height: width > 1000 ? "80vh" : "100%",
          }}
        >
          <Tabs defaultValue="analysis">
            <Tabs.List grow>
              <Tabs.Tab value="analysis" icon={<IconZoomCheck size={16} />}>
                Analysis
              </Tabs.Tab>
              <Tabs.Tab value="database" icon={<IconDatabase size={16} />}>
                Database
              </Tabs.Tab>
              <Tabs.Tab value="annotate" icon={<IconNotes size={16} />}>
                Annotate
              </Tabs.Tab>
              <Tabs.Tab value="info" icon={<IconInfoCircle size={16} />}>
                Info
              </Tabs.Tab>
            </Tabs.List>
            <Tabs.Panel value="info" pt="xs">
              <Stack>
                <GameInfo game={game} />
                <FenInput onSubmit={resetToFen} />
                <PgnInput game={game} />
              </Stack>
            </Tabs.Panel>
            <Tabs.Panel value="database" pt="xs">
              <DatabasePanel makeMove={makeMove} height={boardSize / 2} />
            </Tabs.Panel>
            <Tabs.Panel value="annotate" pt="xs">
              <AnnotationPanel forceUpdate={forceUpdate} setTree={setTree} />
            </Tabs.Panel>
            <Tabs.Panel value="analysis" pt="xs">
              <ScrollArea
                style={{ height: boardSize / 2 }}
                offsetScrollbars
                type="always"
              >
                <Stack>
                  <Accordion
                    variant="separated"
                    multiple
                    chevronSize={0}
                    defaultValue={engines.map((e) => e.path)}
                  >
                    {engines
                      .filter((e) => e.loaded)
                      .map((engine, i) => {
                        return (
                          <Accordion.Item key={engine.path} value={engine.path}>
                            <BestMoves
                              id={i}
                              tab={id}
                              engine={engine}
                              makeMoves={makeMoves}
                              setArrows={setArrows}
                              setTree={setTree}
                            />
                          </Accordion.Item>
                        );
                      })}
                  </Accordion>
                  <EngineSettingsBoard
                    engines={engines}
                    setEngines={setEngines}
                  />
                  <Group grow>
                    <Button
                      variant="default"
                      leftIcon={<IconRobot size={14} />}
                      onClick={() => changeToPlayMode()}
                    >
                      Play against engine
                    </Button>
                    <Button
                      leftIcon={<IconZoomCheck size={14} />}
                      onClick={() => toggleReportingMode()}
                      loading={analysisLoading}
                    >
                      Generate Report
                    </Button>
                  </Group>
                </Stack>
              </ScrollArea>
            </Tabs.Panel>
          </Tabs>
          <Stack>
            <GameNotation
              game={game}
              setTree={setTree}
              deleteVariation={deleteVariation}
              promoteVariation={promoteVariation}
              topVariation={tree.getTopVariation()}
              result={game.result}
              boardSize={width > 1000 ? boardSize : 600}
            />
            <MoveControls
              goToStart={goToStart}
              goToEnd={goToEnd}
              redoMove={redoMove}
              undoMove={undoMove}
            />
          </Stack>
        </Stack>
      </Flex>
    </TreeContext.Provider>
  );
}

export default BoardAnalysis;
