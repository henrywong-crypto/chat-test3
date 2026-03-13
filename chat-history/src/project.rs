use russh_sftp::client::{fs::DirEntry, SftpSession};

// Returns all project directories under ~/.claude/projects on the remote VM.
// Each subdirectory there corresponds to one Claude Code project (named by
// its encoded working directory path) and contains the session JSONL files.
pub(crate) async fn find_all_project_dirs(sftp: &SftpSession, ssh_user_home: &str) -> Vec<String> {
    let projects_base = projects_base_path(ssh_user_home);
    // Directory may not exist yet on a fresh VM; treat as empty rather than an error
    let top_entries: Vec<DirEntry> = sftp
        .read_dir(&projects_base)
        .await
        .map(|entries| entries.collect())
        .unwrap_or_default();
    let mut project_dirs = Vec::new();
    for entry in top_entries {
        let name = entry.file_name();
        if name.starts_with('.') {
            continue;
        }
        let path = format!("{projects_base}/{name}");
        if entry.file_type().is_dir() {
            project_dirs.push(path);
        }
    }
    project_dirs
}

fn projects_base_path(ssh_user_home: &str) -> String {
    format!("{ssh_user_home}/.claude/projects")
}
