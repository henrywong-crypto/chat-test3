pub(crate) fn render_login_page() -> String {
    r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>Web</title>
</head>
<body style="display:flex;align-items:center;justify-content:center;min-height:100vh;margin:0;background:#0f0f0f;color:#fff;font-family:sans-serif">
<div style="text-align:center">
<h1 style="margin-bottom:1.5rem;font-size:1rem;font-weight:bold">Web</h1>
<a href="/login/cognito" style="display:inline-block;padding:0.5rem 1.5rem;background:#3b82f6;color:#fff;border-radius:0.5rem;text-decoration:none">Sign in</a>
</div>
</body>
</html>"#.to_owned()
}

pub(crate) fn render_terminal_page(
    vm_id: &str,
    csrf_token: &str,
    upload_dir: &str,
    has_user_rootfs: bool,
    app_js_version: &str,
    styles_css_version: &str,
) -> String {
    let upload_action = format!("/sessions/{vm_id}/upload");
    let app_js_src = format!("/static/app.js?v={app_js_version}");
    let styles_css_href = format!("/static/styles.css?v={styles_css_version}");
    let has_user_rootfs_str = has_user_rootfs.to_string();
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>Web</title>
<link rel="stylesheet" href="{styles_css_href}"/>
</head>
<body class="flex h-screen overflow-hidden bg-background text-foreground">
<div id="app-config" hidden
  data-vm-id="{vm_id}"
  data-csrf-token="{csrf_token}"
  data-upload-dir="{upload_dir}"
  data-upload-action="{upload_action}"
  data-has-user-rootfs="{has_user_rootfs_str}"
></div>
<div id="app" class="flex h-screen w-screen overflow-hidden"></div>
<script src="{app_js_src}" defer></script>
</body>
</html>"#
    )
}
