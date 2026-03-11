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
        <html lang="en" data-theme="dark">
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
        <html lang="en" data-theme="dark">
            <head>
                <meta charset="utf-8"/>
                <title>"WebCode"</title>
                <link rel="stylesheet" href=styles_css_href/>
            </head>
            <body class="flex flex-col h-screen overflow-hidden">
                <div
                    id="app-config"
                    hidden
                    data-vm-id=vm_id
                    data-csrf-token=csrf_token.clone()
                    data-upload-dir=upload_dir
                    data-upload-action=upload_action
                />
                <Topbar csrf_token=csrf_token has_user_rootfs=has_user_rootfs/>
                <div id="shell-view" class="flex flex-1 min-h-0">
                    <div id="term-container" class="flex-1 min-h-0 bg-black"/>
                    <FilesPanel/>
                </div>
                <div id="chat-view" class="hidden flex-1 min-h-0 flex-col">
                    <ChatView/>
                </div>
                <script src=app_js_src defer/>
            </body>
        </html>
    }
}

#[component]
fn Topbar(csrf_token: String, has_user_rootfs: bool) -> impl IntoView {
    view! {
        <div class="flex items-center justify-between h-10 px-4 bg-base-200 border-b border-base-300 shrink-0">
            <div class="flex items-center gap-3">
                <span class="text-sm font-semibold">"WebCode"</span>
                <div class="flex gap-1">
                    <button id="tab-shell-btn" class="btn btn-xs btn-primary">"Shell"</button>
                    <button id="tab-chat-btn" class="btn btn-xs btn-ghost">"Chat"</button>
                </div>
            </div>
            <div class="flex items-center gap-2">
                {has_user_rootfs.then(|| view! {
                    <button id="reset-btn" type="button" class="btn btn-xs btn-ghost text-error">"Reset"</button>
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
                <button id="files-toggle-btn" class="btn btn-xs btn-ghost">"Files"</button>
                <a href="/logout" class="btn btn-xs btn-ghost">"Logout"</a>
            </div>
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
fn ChatView() -> impl IntoView {
    view! {
        <div class="flex items-center justify-between px-4 py-2 border-b border-gray-700 shrink-0" style="background:#1f2937">
            <div class="flex items-center gap-2">
                <div class="w-6 h-6 rounded-full flex items-center justify-center text-xs text-white font-bold" style="background:#f97316">"C"</div>
                <span class="text-sm font-semibold text-gray-100">"Claude"</span>
            </div>
            <div class="flex items-center gap-1">
                <button id="chat-new-btn" class="btn btn-xs btn-ghost text-gray-400">"New"</button>
                <button id="chat-history-btn" class="btn btn-xs btn-ghost text-gray-400">"History"</button>
            </div>
        </div>
        <div id="chat-sessions-panel" class="hidden border-b border-gray-700 overflow-y-auto" style="max-height:200px;background:#1f2937"/>
        <div id="chat-scroll" class="flex-1 overflow-y-auto" style="background:#111827">
            <div id="chat-messages" class="max-w-3xl mx-auto py-4 px-4 space-y-3"/>
        </div>
        <div class="border-t border-gray-700 shrink-0" style="background:#1f2937">
            <div class="max-w-3xl mx-auto px-4 py-3">
                <div id="chat-attachments" class="hidden flex-wrap gap-1 pb-2"/>
                <div class="flex items-end gap-2">
                    <button id="chat-attach-btn" class="btn btn-xs btn-ghost text-gray-400 shrink-0" title="Attach file">"📎"</button>
                    <input type="file" id="chat-attach-input" class="hidden" multiple=true />
                    <textarea
                        id="chat-input"
                        class="flex-1 resize-none rounded-xl border border-gray-600 px-3 py-2 text-sm text-gray-100 placeholder-gray-500 focus:outline-none focus:border-blue-500"
                        style="background:#374151;min-height:40px;max-height:160px"
                        rows="1"
                        placeholder="Message Claude\u{2026}"
                    />
                    <button id="chat-stop-btn" class="btn btn-sm rounded-xl hidden" style="background:#dc2626;color:white;border:none" title="Stop (Esc)">"■ Stop"</button>
                    <button id="chat-send-btn" class="btn btn-sm rounded-xl" style="background:#2563eb;color:white;border:none">"Send"</button>
                </div>
            </div>
        </div>
    }
}
