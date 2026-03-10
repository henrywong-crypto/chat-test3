use leptos::prelude::*;

pub(crate) fn render_login_page() -> String {
    Owner::new().with(|| view! { <LoginPage /> }.to_html())
}

pub(crate) fn render_terminal_page(
    vm_id: &str,
    csrf_token: &str,
    upload_dir: &str,
    has_user_rootfs: bool,
) -> String {
    let vm_id = vm_id.to_owned();
    let csrf_token = csrf_token.to_owned();
    let upload_dir = upload_dir.to_owned();
    Owner::new().with(move || {
        view! {
            <TerminalPage
                vm_id=vm_id
                csrf_token=csrf_token
                upload_dir=upload_dir
                has_user_rootfs=has_user_rootfs
            />
        }
        .to_html()
    })
}

#[component]
fn LoginPage() -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <title>"WebCode"</title>
                <link rel="stylesheet" href="/static/styles.css"/>
            </head>
            <body class="login-page">
                <div class="login-card">
                    <h1>"WebCode"</h1>
                    <a href="/login/cognito" class="btn btn-primary">"Sign in with Cognito"</a>
                </div>
            </body>
        </html>
    }
}

#[component]
fn TerminalPage(
    vm_id: String,
    csrf_token: String,
    upload_dir: String,
    has_user_rootfs: bool,
) -> impl IntoView {
    let short_id = format!("{}…", vm_id.get(..8).unwrap_or(&vm_id));
    let upload_action = format!("/sessions/{vm_id}/upload");
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <title>"WebCode — " {short_id.clone()}</title>
                <link rel="stylesheet" href="/static/styles.css"/>
            </head>
            <body class="terminal-page">
                <div
                    id="app-config"
                    hidden
                    data-vm-id=vm_id
                    data-csrf-token=csrf_token.clone()
                    data-upload-dir=upload_dir
                    data-upload-action=upload_action
                />
                <TerminalTopbar short_id=short_id csrf_token=csrf_token has_user_rootfs=has_user_rootfs/>
                <div id="main-area">
                    <div id="term-container"/>
                    <FilesPanel/>
                </div>
                <script src="/static/terminal.js" defer/>
                <script src="/static/file-manager.js" defer/>
            </body>
        </html>
    }
}

#[component]
fn TerminalTopbar(short_id: String, csrf_token: String, has_user_rootfs: bool) -> impl IntoView {
    view! {
        <div id="topbar">
            <div id="topbar-left">
                <span class="topbar-title" style="font-size:14px;font-weight:600">"WebCode"</span>
                <span id="vm-id">{short_id}</span>
            </div>
            <div id="topbar-right">
                {has_user_rootfs.then(|| view! {
                    <form method="post" action="/rootfs/delete">
                        <input type="hidden" name="csrf_token" value=csrf_token/>
                        <button type="submit" class="btn btn-ghost">"Reset"</button>
                    </form>
                })}
                <button class="btn" onclick="toggleFiles()">"📁 Files"</button>
                <a href="/logout" class="btn btn-ghost">"Logout"</a>
            </div>
        </div>
    }
}

#[component]
fn FilesPanel() -> impl IntoView {
    view! {
        <div id="files-panel">
            <div id="files-header">
                <span id="files-breadcrumb"/>
                <button class="btn btn-ghost" style="padding:2px 6px" onclick="toggleFiles()">"✕"</button>
            </div>
            <div id="files-list"/>
            <div id="files-footer">
                <input type="file" id="fm-file-input" style="display:none"/>
                <label for="fm-file-input" class="btn" style="flex:1;justify-content:center">"↑ Upload here"</label>
                <span id="files-upload-status"/>
            </div>
        </div>
    }
}
