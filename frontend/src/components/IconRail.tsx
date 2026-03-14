import React from "react";
import { MessageSquare, Terminal, FolderOpen, LogOut, RotateCcw } from "lucide-react";
import type { ViewTab } from "../types";

interface IconRailProps {
  activeTab: ViewTab;
  onTabChange: (tab: ViewTab) => void;
  hasUserRootfs: boolean;
  csrfToken: string;
}

export default function IconRail({ activeTab, onTabChange, hasUserRootfs, csrfToken }: IconRailProps) {
  return (
    <div className="flex w-12 flex-col items-center gap-1 border-r border-border bg-card py-2">
      <IconButton
        active={activeTab === "chat"}
        title="Chat"
        onClick={() => onTabChange("chat")}
      >
        <MessageSquare className="h-5 w-5" />
      </IconButton>
      <IconButton
        active={activeTab === "terminal"}
        title="Terminal"
        onClick={() => onTabChange("terminal")}
      >
        <Terminal className="h-5 w-5" />
      </IconButton>
      <IconButton
        active={activeTab === "files"}
        title="Files"
        onClick={() => onTabChange("files")}
      >
        <FolderOpen className="h-5 w-5" />
      </IconButton>

      <div className="mt-auto flex flex-col items-center gap-1">
        {hasUserRootfs && <ResetButton csrfToken={csrfToken} />}
        <IconButton title="Logout" onClick={() => { window.location.href = "/logout"; }}>
          <LogOut className="h-5 w-5" />
        </IconButton>
      </div>
    </div>
  );
}

function IconButton({
  active,
  title,
  onClick,
  children,
}: {
  active?: boolean;
  title: string;
  onClick?: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      title={title}
      onClick={onClick}
      className={`flex h-10 w-10 items-center justify-center rounded-lg transition-colors ${
        active
          ? "bg-primary/20 text-primary"
          : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
      }`}
    >
      {children}
    </button>
  );
}

function ResetButton({ csrfToken }: { csrfToken: string }) {
  const [open, setOpen] = React.useState(false);

  return (
    <>
      <IconButton title="Reset environment" onClick={() => setOpen(true)}>
        <RotateCcw className="h-5 w-5" />
      </IconButton>

      {open && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={() => setOpen(false)}>
          <div
            className="mx-4 w-full max-w-sm rounded-xl border border-border bg-card p-6 shadow-2xl"
            onClick={(e) => e.stopPropagation()}
          >
            <h3 className="mb-2 text-base font-semibold text-foreground">Reset Environment?</h3>
            <p className="mb-1 text-sm text-muted-foreground">
              This will permanently delete all your files and reset your workspace to a clean state.
            </p>
            <p className="mb-6 text-sm font-medium text-destructive">
              Please backup your files before proceeding. This action cannot be undone.
            </p>
            <div className="flex justify-end gap-3">
              <button
                onClick={() => setOpen(false)}
                className="rounded-lg px-4 py-2 text-sm font-medium text-muted-foreground transition-colors hover:bg-accent"
              >
                Cancel
              </button>
              <form method="post" action="/rootfs/delete" onSubmit={() => setOpen(false)}>
                <input type="hidden" name="csrf_token" value={csrfToken} />
                <button
                  type="submit"
                  className="rounded-lg bg-destructive px-4 py-2 text-sm font-medium text-destructive-foreground transition-colors hover:bg-destructive/90"
                >
                  Reset Environment
                </button>
              </form>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
