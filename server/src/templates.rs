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
            <body class="login-bg flex items-center justify-center min-h-screen">
                <div class="card login-card w-80 shadow-xl">
                    <div class="card-body gap-6">
                        <h1 class="font-bold text-sm">"Web"</h1>
                        <a href="/login/cognito" class="btn login-btn">"Sign in"</a>
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
            <body class="flex h-screen overflow-hidden">
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
                        <div id="api-key-banner" class="hidden items-center gap-2 px-4 py-2 text-xs border-b border-base-300">
                            <span class="opacity-60">"No API key configured."</span>
                            <button id="api-key-banner-btn" class="underline opacity-80 hover:opacity-100">"Configure now"</button>
                        </div>
                        <div id="chat-scroll" class="flex-1 overflow-y-auto">
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
            <button id="tab-chat-icon" class="icon-btn icon-active" title="Chat">
                <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/>
                    <circle cx="9" cy="11" r="0.5" fill="currentColor"/>
                    <circle cx="12" cy="11" r="0.5" fill="currentColor"/>
                    <circle cx="15" cy="11" r="0.5" fill="currentColor"/>
                </svg>
            </button>
            <button id="tab-shell-icon" class="icon-btn" title="Terminal & Files">
                <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <rect x="3" y="3" width="18" height="18" rx="2" ry="2"/>
                    <path d="M7 8l3 3-3 3"/>
                    <path d="M13 14h4"/>
                </svg>
            </button>
            <div class="icon-rail-spacer"/>
            <button id="settings-btn" class="icon-btn" title="API settings">
                <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="12" cy="12" r="3"/>
                    <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
                </svg>
            </button>
            <dialog id="settings-dialog" class="modal">
                <div class="modal-box" style="max-width:420px">
                    <h3 class="font-bold text-sm mb-4">"API Settings"</h3>
                    <div id="settings-bedrock-msg" class="hidden text-xs opacity-60 mb-3">
                        "Using AWS Bedrock — no API key required."
                    </div>
                    <div id="settings-api-key-form">
                        <div class="mb-3">
                            <label class="text-xs font-medium opacity-60 block mb-1">"Base URL"</label>
                            <div id="settings-base-url" class="text-xs font-mono opacity-50 break-all">"https://api.anthropic.com"</div>
                        </div>
                        <div class="mb-4">
                            <label for="settings-api-key-input" class="text-xs font-medium opacity-60 block mb-1">"API Key"</label>
                            <input
                                id="settings-api-key-input"
                                type="password"
                                placeholder="sk-ant-..."
                                class="input input-bordered input-sm w-full font-mono text-xs"
                            />
                        </div>
                        <div id="settings-save-status" class="text-xs mb-2 hidden"/>
                    </div>
                    <div class="modal-action mt-2">
                        <form method="dialog">
                            <button class="btn btn-sm btn-ghost">"Cancel"</button>
                        </form>
                        <button id="settings-save-btn" class="btn btn-sm btn-primary">"Save"</button>
                    </div>
                </div>
                <form method="dialog" class="modal-backdrop">
                    <button>"close"</button>
                </form>
            </dialog>
            <button id="theme-toggle-btn" class="icon-btn" title="Toggle theme">
                <span class="theme-icon sun-icon">
                    <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <circle cx="12" cy="12" r="5"/>
                        <line x1="12" y1="1" x2="12" y2="3"/>
                        <line x1="12" y1="21" x2="12" y2="23"/>
                        <line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/>
                        <line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/>
                        <line x1="1" y1="12" x2="3" y2="12"/>
                        <line x1="21" y1="12" x2="23" y2="12"/>
                        <line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/>
                        <line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/>
                    </svg>
                </span>
                <span class="theme-icon moon-icon">
                    <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/>
                    </svg>
                </span>
            </button>
            {has_user_rootfs.then(|| view! {
                <button id="reset-btn" class="icon-btn reset-icon-btn" title="Reset environment">
                    <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M23 4v6h-6"/>
                        <path d="M1 20v-6h6"/>
                        <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10"/>
                        <path d="M20.49 15a9 9 0 0 1-14.85 3.36L1 14"/>
                    </svg>
                </button>
                <dialog id="reset-dialog" class="modal reset-dialog">
                    <div class="modal-box reset-modal-box">
                        <div class="reset-warning-icon">
                            <svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                <circle cx="12" cy="12" r="10"></circle>
                                <line x1="12" y1="8" x2="12" y2="12"></line>
                                <line x1="12" y1="16" x2="12.01" y2="16"></line>
                            </svg>
                        </div>
                        <h3 class="reset-dialog-title">"Reset Environment?"</h3>
                        <p class="reset-dialog-text">"This will permanently delete all your files and reset your workspace to a clean state."</p>
                        <p class="reset-dialog-warning">"Please backup your files before proceeding. This action cannot be undone."</p>
                        <div class="modal-action reset-modal-actions">
                            <form method="dialog">
                                <button class="btn btn-sm reset-cancel-btn">"Cancel"</button>
                            </form>
                            <form method="post" action="/rootfs/delete">
                                <input type="hidden" name="csrf_token" value=csrf_token/>
                                <button type="submit" class="btn btn-sm reset-confirm-btn">"Reset Environment"</button>
                            </form>
                        </div>
                    </div>
                    <form method="dialog" class="modal-backdrop">
                        <button>"close"</button>
                    </form>
                </dialog>
            })}
            <a href="/logout" class="icon-btn" title="Logout">
                <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/>
                    <polyline points="16 17 21 12 16 7"/>
                    <line x1="21" y1="12" x2="9" y2="12"/>
                </svg>
            </a>
        </div>
    }
}

#[component]
fn SessionPanel() -> impl IntoView {
    view! {
        <div class="session-panel">
            <div class="session-panel-header">
                <span>"Chats"</span>
                <button id="chat-history-refresh-btn" class="icon-btn" style="width:28px;height:28px;" title="Refresh">
                    <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M21.5 2v6h-6M2.5 22v-6h6M2 11.5a10 10 0 0 1 18.8-4.3M22 12.5a10 10 0 0 1-18.8 4.2"/>
                    </svg>
                </button>
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
            <button id="files-toggle-btn" class="icon-btn" style="width:32px;height:32px;display:none" title="Show files">
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path>
                </svg>
            </button>
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
                <label for="fm-file-input" class="btn btn-outline btn-xs flex-1 files-upload-btn">
                    <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path>
                        <polyline points="17 8 12 3 7 8"></polyline>
                        <line x1="12" y1="3" x2="12" y2="15"></line>
                    </svg>
                    <span>"Upload"</span>
                </label>
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
                <button id="chat-attach-btn" class="icon-btn" style="width:32px;height:32px;flex-shrink:0" title="Attach file">
                    <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                        <path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"></path>
                    </svg>
                </button>
                <input type="file" id="chat-attach-input" class="hidden" multiple=true/>
                <textarea
                    id="chat-input"
                    rows="1"
                    placeholder="Message Claude\u{2026}"
                />
                <button id="chat-stop-btn" class="send-btn hidden" title="Stop (Esc)" style="background:#ef4444">"■"</button>
                <button id="chat-send-btn" class="send-btn" title="Send">"↑"</button>
            </div>
        </div>
    }
}
