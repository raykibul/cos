fn main() {
    // Listing the app's commands gives each an `allow-…` permission, so the
    // capability in `capabilities/` names exactly the ones the UI may call.
    let commands = &[
        "daemon_info",
        "daemon_request",
        "daemon_upload",
        "daemon_fetch_bytes",
        "follow_events",
        "attach_terminal",
        "attach_shell",
        "close_stream",
        "hosts_list",
        "host_add",
        "host_rename",
        "host_remove",
        "host_rename",
        "host_select",
        "tailnet_peers",
        "set_waiting_count",
        "notify",
        "reveal_path",
        "open_in_terminal",
        "open_url",
    ];
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(commands)),
    )
    .expect("failed to run the Tauri build script");
}
