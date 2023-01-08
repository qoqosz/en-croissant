import {
  ActionIcon,
  createStyles,
  Group,
  ScrollArea,
  Stack,
  Tabs
} from "@mantine/core";
import { useHotkeys, useSessionStorage } from "@mantine/hooks";
import { IconPlus } from "@tabler/icons";
import BoardAnalysis from "../boards/BoardAnalysis";
import { BoardTab } from "./BoardTab";

export interface Tab {
  name: string;
  value: string;
}

const useStyles = createStyles((theme) => ({
  newTab: {
    backgroundColor: theme.colors.dark[7],
    ":hover": {
      backgroundColor: theme.colors.dark[6],
    },
  },
}));

export function genID() {
  var S4 = function () {
    return (((1 + Math.random()) * 0x10000) | 0).toString(16).substring(1);
  };
  return S4() + S4();
}

export default function BoardsPage() {
  const { classes } = useStyles();
  const firstId = genID();
  const [tabs, setTabs] = useSessionStorage<Tab[]>({
    key: "tabs",
    defaultValue: [],
  });
  const [activeTab, setActiveTab] = useSessionStorage<string | null>({
    key: "activeTab",
    defaultValue: firstId,
  });

  function createTab() {
    const id = genID();

    setTabs((prev) => [
      ...prev,
      {
        name: "New tab",
        value: id,
      },
    ]);
    setActiveTab(id);
    return id;
  }

  function closeTab(value: string | null) {
    if (value !== null) {
      if (value === activeTab) {
        const index = tabs.findIndex((tab) => tab.value === value);
        if (tabs.length > 1) {
          if (index === tabs.length - 1) {
            setActiveTab(tabs[index - 1].value);
          } else {
            setActiveTab(tabs[index + 1].value);
          }
        } else {
          setActiveTab(null);
        }
      }
      setTabs((prev) => prev.filter((tab) => tab.value !== value));
    }
  }

  function selectTab(index: number) {
    setActiveTab(tabs[Math.min(index, tabs.length - 1)].value);
  }

  function cycleTabs(reverse = false) {
    const index = tabs.findIndex((tab) => tab.value === activeTab);
    if (reverse) {
      if (index === 0) {
        setActiveTab(tabs[tabs.length - 1].value);
      } else {
        setActiveTab(tabs[index - 1].value);
      }
    } else {
      if (index === tabs.length - 1) {
        setActiveTab(tabs[0].value);
      } else {
        setActiveTab(tabs[index + 1].value);
      }
    }
  }

  function renameTab(value: string, name: string) {
    setTabs((prev) =>
      prev.map((tab) => {
        if (tab.value === value) {
          return { ...tab, name };
        }
        return tab;
      })
    );
  }

  function duplicateTab(value: string) {
    const id = genID();
    const tab = tabs.find((tab) => tab.value === value);
    if (sessionStorage.getItem(value)) {
      sessionStorage.setItem(id, sessionStorage.getItem(value) || "");
    }

    if (tab) {
      setTabs((prev) => [
        ...prev,
        {
          name: tab.name,
          value: id,
        },
      ]);
      setActiveTab(id);
    }
  }

  useHotkeys([
    ["ctrl+T", () => createTab()],
    ["ctrl+W", () => closeTab(activeTab)],
    ["ctrl+tab", () => cycleTabs()],
    ["ctrl+shift+tab", () => cycleTabs(true)],
    ["alt+1", () => selectTab(0)],
    ["ctrl+1", () => selectTab(0)],
    ["alt+2", () => selectTab(1)],
    ["ctrl+2", () => selectTab(1)],
    ["alt+3", () => selectTab(2)],
    ["ctrl+3", () => selectTab(2)],
    ["alt+4", () => selectTab(3)],
    ["ctrl+4", () => selectTab(3)],
    ["alt+5", () => selectTab(4)],
    ["ctrl+5", () => selectTab(4)],
    ["alt+6", () => selectTab(5)],
    ["ctrl+6", () => selectTab(5)],
    ["alt+7", () => selectTab(6)],
    ["ctrl+7", () => selectTab(6)],
    ["alt+8", () => selectTab(7)],
    ["ctrl+8", () => selectTab(7)],
    ["alt+9", () => selectTab(tabs.length - 1)],
    ["ctrl+9", () => selectTab(tabs.length - 1)],
  ]);

  return (
    <>
      <Stack>
        <Group grow>
          <Tabs
            value={activeTab}
            onTabChange={(v) => setActiveTab(v)}
            variant="outline"
          >
            <ScrollArea offsetScrollbars>
              <Group
                spacing={0}
                sx={{ flexWrap: "nowrap", overflowY: "hidden", zIndex: 100 }}
              >
                <Tabs.List sx={{ flexWrap: "nowrap", overflowY: "hidden", zIndex: 100 }}>
                  {tabs.map((tab) => (
                    <BoardTab
                      key={tab.value}
                      tab={tab}
                      closeTab={closeTab}
                      renameTab={renameTab}
                      duplicateTab={duplicateTab}
                      selected={activeTab === tab.value}
                    />
                  ))}
                </Tabs.List>
                <ActionIcon
                  onClick={() => createTab()}
                  className={classes.newTab}
                >
                  <IconPlus size={16} />
                </ActionIcon>
              </Group>
            </ScrollArea>

            {tabs.map((tab) => (
              <Tabs.Panel
                key={tab.value}
                value={tab.value}
                sx={{ zIndex: -100 }}
              >
                <BoardAnalysis id={tab.value} />
              </Tabs.Panel>
            ))}
          </Tabs>
        </Group>
      </Stack>
    </>
  );
}
