import React, { useCallback, useEffect, useState } from "react";
import { SseProvider, useSse } from "./contexts/SseContext";
import IconRail from "./components/IconRail";
import Sidebar from "./components/Sidebar";
import ChatInterface from "./components/ChatInterface";
import Terminal from "./components/Terminal";
import FileManager from "./components/FileManager";
import SettingsPanel from "./components/SettingsPanel";
import type { ChatSession, ViewTab } from "./types";

class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  constructor(props: { children: React.ReactNode }) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  render() {
    if (this.state.error) {
      return (
        <div className="flex h-screen w-screen items-center justify-center bg-background text-foreground">
          <div className="max-w-md space-y-2 p-6 text-center">
            <p className="text-sm font-medium">Something went wrong</p>
            <p className="font-mono text-xs text-muted-foreground">{this.state.error.message}</p>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

function AppContent() {
  const { hasUserRootfs, csrfToken, loadHistory, deleteSession } = useSse();
  const [activeTab, setActiveTab] = useState<ViewTab>("chat");
  const [sessions, setSessions] = useState<ChatSession[]>([]);
  const [selectedSession, setSelectedSession] = useState<ChatSession | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [runningSessionId, setRunningSessionId] = useState<string | null>(null);
  const [darkMode, setDarkMode] = useState<boolean>(() => {
    const saved = localStorage.getItem("ui-theme");
    return saved ? saved === "dark" : true;
  });

  useEffect(() => {
    if (darkMode) {
      document.documentElement.classList.remove("light");
    } else {
      document.documentElement.classList.add("light");
    }
    localStorage.setItem("ui-theme", darkMode ? "dark" : "light");
  }, [darkMode]);

  const toggleDarkMode = useCallback(() => setDarkMode((m) => !m), []);

  useEffect(() => {
    loadHistory().then(setSessions).catch(console.error);
  }, [loadHistory, setSessions]);

  const handleDeleteSession = useCallback(async (session: ChatSession) => {
    if (!session.project_dir) return;
    try {
      await deleteSession(session.session_id, session.project_dir);
      setSessions((prev) => prev.filter((s) => s.session_id !== session.session_id));
      if (selectedSession?.session_id === session.session_id) {
        setSelectedSession(null);
      }
    } catch (err) {
      console.error("Failed to delete session", err);
    }
  }, [deleteSession, selectedSession]);

  const handleRefresh = useCallback(() => {
    loadHistory().then(setSessions).catch(console.error);
  }, [loadHistory]);

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-background">
      <IconRail
        activeTab={activeTab}
        onTabChange={setActiveTab}
        hasUserRootfs={hasUserRootfs}
        csrfToken={csrfToken}
        onSettingsOpen={() => setShowSettings(true)}
        darkMode={darkMode}
        onToggleDarkMode={toggleDarkMode}
      />

      {activeTab === "chat" && (
        <Sidebar
          sessions={sessions}
          viewSessionId={selectedSession?.session_id ?? null}
          runningSessionId={runningSessionId}
          onSelectSession={setSelectedSession}
          onNewChat={() => {
            setSelectedSession({
              session_id: crypto.randomUUID(),
              created_at: new Date().toISOString(),
              title: "New chat\u2026",
              is_pending: true,
            });
          }}
          onRefresh={handleRefresh}
          onDeleteSession={handleDeleteSession}
        />
      )}

      <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
        {activeTab === "chat" && (
          <ChatInterface
            sessions={sessions}
            setSessions={setSessions}
            selectedSession={selectedSession}
            onRunningSessionChange={setRunningSessionId}
          />
        )}
        <div
          style={{ display: activeTab === "terminal" ? "flex" : "none" }}
          className="min-h-0 flex-1 flex-col"
        >
          <Terminal visible={activeTab === "terminal"} />
        </div>
        <div
          style={{ display: activeTab === "files" ? "flex" : "none" }}
          className="min-h-0 flex-1 flex-col"
        >
          <FileManager />
        </div>
      </main>

      {showSettings && <SettingsPanel onClose={() => setShowSettings(false)} />}
    </div>
  );
}

export default function App() {
  return (
    <ErrorBoundary>
      <SseProvider>
        <AppContent />
      </SseProvider>
    </ErrorBoundary>
  );
}
