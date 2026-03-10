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
                        <h1 class="card-title text-sm">"WebCode"</h1>
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
    let short_id = format!("{}…", vm_id.get(..8).unwrap_or(&vm_id));
    let upload_action = format!("/sessions/{vm_id}/upload");
    let app_js_src = format!("/static/app.js?v={app_js_version}");
    let styles_css_href = format!("/static/styles.css?v={styles_css_version}");
    view! {
        <!DOCTYPE html>
        <html lang="en" data-theme="dark">
            <head>
                <meta charset="utf-8"/>
                <title>"WebCode — " {short_id.clone()}</title>
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
                <TerminalTopbar short_id=short_id csrf_token=csrf_token has_user_rootfs=has_user_rootfs/>
                <div class="flex flex-1 min-h-0">
                    <div id="term-container" class="flex-1 min-h-0 bg-black"/>
                    <FilesPanel/>
                </div>
                <script src=app_js_src defer/>
            </body>
        </html>
    }
}

#[component]
fn TerminalTopbar(short_id: String, csrf_token: String, has_user_rootfs: bool) -> impl IntoView {
    view! {
        <div class="flex items-center justify-between h-10 px-4 bg-base-200 border-b border-base-300 shrink-0">
            <div class="flex items-center gap-3">
                <span class="text-sm font-semibold">"WebCode"</span>
                <span class="text-xs opacity-50">{short_id}</span>
            </div>
            <div class="flex items-center gap-2">
                {has_user_rootfs.then(|| view! {
                    <form method="post" action="/rootfs/delete">
                        <input type="hidden" name="csrf_token" value=csrf_token/>
                        <button type="submit" class="btn btn-xs btn-ghost">"Reset"</button>
                    </form>
                })}
                <button id="files-toggle-btn" class="btn btn-xs btn-ghost">"📁 Files"</button>
                <a href="/logout" class="btn btn-xs btn-ghost">"Logout"</a>
            </div>
        </div>
    }
}

#[component]
fn FilesPanel() -> impl IntoView {
    view! {
        <div id="files-panel" class="hidden w-64 shrink-0 flex-col bg-base-200 border-l border-base-300 overflow-hidden">
            <div class="flex items-center justify-between px-3 py-2 border-b border-base-300 shrink-0">
                <span id="files-breadcrumb" class="text-xs opacity-50 truncate flex-1"/>
                <button id="files-close-btn" class="btn btn-xs btn-ghost px-1">"✕"</button>
            </div>
            <div id="files-list" class="flex-1 overflow-y-auto"/>
            <div class="flex items-center gap-2 px-3 py-2 border-t border-base-300 shrink-0">
                <input type="file" id="fm-file-input" class="hidden"/>
                <label for="fm-file-input" class="btn btn-xs flex-1 justify-center">"↑ Upload here"</label>
                <span id="files-upload-status" class="text-xs"/>
            </div>
        </div>
    }
}
