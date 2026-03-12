use leptos::prelude::*;

pub(crate) fn render_login_page() -> String {
    Owner::new().with(|| view! { <LoginPage /> }.to_html())
}

pub(crate) fn render_terminal_page(
    vm_id: &str,
    csrf_token: &str,
    upload_dir: &str,
    has_user_rootfs: bool,
    app_js_version: &str,
    styles_css_version: &str,
) -> String {
    let vm_id = vm_id.to_owned();
    let csrf_token = csrf_token.to_owned();
    let upload_dir = upload_dir.to_owned();
    let app_js_version = app_js_version.to_owned();
    let styles_css_version = styles_css_version.to_owned();
    Owner::new().with(move || {
        view! {
            <TerminalPage
                vm_id=vm_id
                csrf_token=csrf_token
                upload_dir=upload_dir
                has_user_rootfs=has_user_rootfs
                app_js_version=app_js_version
                styles_css_version=styles_css_version
            />
        }
        .to_html()
    })
}

#[component]
fn LoginPage() -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en" data-theme="light">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <title>"WebCode"</title>
                <link rel="stylesheet" href="/static/styles.css"/>
            </head>
            <body class="bg-base-100 flex items-center justify-center min-h-screen">
                <div class="card bg-base-200 w-80 shadow-xl">
                    <div class="card-body gap-6">
                        <h1 class="font-bold text-sm">"WebCode"</h1>
                        <a href="/login/cognito" class="btn btn-primary">"Sign in with Cognito"</a>
                    </div>
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
    app_js_version: String,
    styles_css_version: String,
) -> impl IntoView {
    let upload_action = format!("/sessions/{vm_id}/upload");
    let app_js_src = format!("/static/app.js?v={app_js_version}");
    let styles_css_href = format!("/static/styles.css?v={styles_css_version}");
    view! {
        <!DOCTYPE html>
        <html lang="en" data-theme="light">
            <head>
                <meta charset="utf-8"/>
                <title>"WebCode"</title>
                <link rel="stylesheet" href=styles_css_href/>
            </head>
            <body class="flex h-screen overflow-hidden bg-white text-gray-900">
                <div
                    id="app-config"
                    hidden
                    data-vm-id=vm_id
                    data-csrf-token=csrf_token.clone()
                    data-upload-dir=upload_dir
                    data-upload-action=upload_action
                />
                <IconRail csrf_token=csrf_token has_user_rootfs=has_user_rootfs/>
                <SessionPanel/>
                <div id="main-panel" class="flex flex-col flex-1 min-w-0">
                    <ChatHeader/>
                    <div id="shell-view" class="hidden flex-1 min-h-0 flex">
                        <div id="term-container" class="flex-1 min-h-0 bg-black"/>
                        <FilesPanel/>
                    </div>
                    <div id="chat-view" class="flex flex-col flex-1 min-h-0">
                        <div id="chat-scroll" class="flex-1 overflow-y-auto bg-white">
                            <div id="chat-messages" class="max-w-3xl mx-auto py-4 px-4 space-y-3"/>
                        </div>
                        <ChatInputArea/>
                    </div>
                </div>
                <script src=app_js_src defer/>
            </body>
        </html>
    }
}

#[component]
fn IconRail(csrf_token: String, has_user_rootfs: bool) -> impl IntoView {
    view! {
        <div class="icon-rail">
            <div class="traffic-lights">
                <div class="traffic-light" style="background:#ff5f57"/>
                <div class="traffic-light" style="background:#febc2e"/>
                <div class="traffic-light" style="background:#28c840"/>
            </div>
            <button id="tab-chat-icon" class="icon-btn icon-active" title="Chat">"💬"</button>
            <button id="tab-shell-icon" class="icon-btn" title="Shell">"⌨"</button>
            <button id="tab-files-icon" class="icon-btn" title="Files">"📁"</button>
            <div class="icon-rail-spacer"/>
            {has_user_rootfs.then(|| view! {
                <button id="reset-btn" class="icon-btn" title="Reset environment" style="color:#ef4444">"⟳"</button>
                <dialog id="reset-dialog" class="modal">
                    <div class="modal-box">
                        <p class="font-semibold text-sm mb-3">"Reset to base environment?"</p>
                        <p class="text-sm opacity-70 mb-4">"Your current session will end and a fresh VM will start from the base image."</p>
                        <div class="modal-action mt-0">
                            <form method="dialog">
                                <button class="btn btn-sm btn-ghost">"Cancel"</button>
                            </form>
                            <form method="post" action="/rootfs/delete">
                                <input type="hidden" name="csrf_token" value=csrf_token/>
                                <button type="submit" class="btn btn-sm btn-error">"Reset"</button>
                            </form>
                        </div>
                    </div>
                    <form method="dialog" class="modal-backdrop">
                        <button>"close"</button>
                    </form>
                </dialog>
            })}
            <a href="/logout" class="icon-btn" title="Logout">"⎋"</a>
        </div>
    }
}

#[component]
fn SessionPanel() -> impl IntoView {
    view! {
        <div class="session-panel">
            <div class="session-panel-header">
                <span>"Chats"</span>
                <button id="chat-history-refresh-btn" class="icon-btn" style="width:24px;height:24px;font-size:12px" title="Refresh">"↺"</button>
            </div>
            <div id="chat-sessions-list" class="flex-1 overflow-y-auto py-1"/>
            <div class="session-panel-footer">
                <button id="chat-new-btn" class="footer-btn">"+ New Chat"</button>
            </div>
        </div>
    }
}

#[component]
fn ChatHeader() -> impl IntoView {
    view! {
        <div class="chat-header">
            <div id="chat-title" class="chat-title">"New Chat"</div>
        </div>
    }
}

#[component]
fn FilesPanel() -> impl IntoView {
    view! {
        <div id="files-panel" class="hidden w-64 shrink-0 flex-col bg-base-200 border-l border-base-300 overflow-hidden">
            <div class="border-b border-base-300 shrink-0">
                <div class="flex items-center justify-between px-3 py-2">
                    <span class="font-semibold text-sm">"Files"</span>
                    <button id="files-close-btn" class="btn btn-xs btn-ghost btn-square">"✕"</button>
                </div>
                <div id="files-breadcrumb" class="text-xs opacity-50 px-3 pb-2 truncate"/>
            </div>
            <div id="files-list" class="flex-1 overflow-y-auto"/>
            <div class="flex items-center gap-2 px-3 py-2 border-t border-base-300 shrink-0">
                <input type="file" id="fm-file-input" class="hidden"/>
                <label for="fm-file-input" class="btn btn-outline btn-xs flex-1">"↑ Upload here"</label>
                <span id="files-upload-status" class="text-xs"/>
            </div>
        </div>
    }
}

#[component]
fn ChatInputArea() -> impl IntoView {
    view! {
        <div class="input-area">
            <div id="chat-attachments" class="hidden flex-wrap gap-1 pb-2"/>
            <div class="input-row">
                <button id="chat-attach-btn" class="icon-btn" style="width:28px;height:28px;font-size:14px;flex-shrink:0" title="Attach file">"📎"</button>
                <input type="file" id="chat-attach-input" class="hidden" multiple=true/>
                <textarea
                    id="chat-input"
                    rows="1"
                    placeholder="Message Claude\u{2026}"
                />
                <button id="chat-stop-btn" class="stop-btn hidden" title="Stop (Esc)">"■"</button>
                <button id="chat-send-btn" class="send-btn" title="Send">"↑"</button>
            </div>
        </div>
    }
}
