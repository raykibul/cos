fn main() {
    // Listing the app's commands gives each an `allow-…` permission, so the
    // capability in `capabilities/` names exactly the ones the UI may call.
    let commands = &[
        "daemon_info",
        "daemon_request",
        "follow_events",
        "attach_terminal",
        "close_stream",
        "hosts_list",
        "host_add",
        "host_remove",
        "host_select",
        "tailnet_peers",
        "set_waiting_count",
        "notify",
        "reveal_path",
        "open_in_terminal",
    ];
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(commands)),
    )
    .expect("failed to run the Tauri build script");
}
