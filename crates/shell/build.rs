/// Every command of the `octopus` plugin, each granted by its `default` permission.
const COMMANDS: &[&str] = &[
    "boot",
    "ready",
    "ui_act",
    "quit",
    "server_pair",
    "server_offer",
    "server_clients",
    "server_revoke",
    "servers_refresh",
    "server_rename",
    "server_forget",
    "workspace_add",
    "workspace_remove",
    "section_rename",
    "section_color",
    "auto_color",
    "task_focus",
    "task_move",
    "task_rename",
    "task_archive",
    "task_create",
    "term_attach",
    "term_input",
    "term_detach",
    "term_open",
    "term_close",
    "term_claim",
    "term_take_lock",
    "term_resize",
    "file_list",
    "file_read",
    "file_write",
    "file_create",
    "file_move",
    "file_copy",
    "file_delete",
    "file_upload",
    "file_search",
];

fn main() {
    let allowed: String =
        COMMANDS.iter().map(|command| format!("  \"allow-{}\",\n", command.replace('_', "-"))).collect();
    let default =
        format!("[default]\ndescription = \"Every command of the client app's core.\"\npermissions = [\n{allowed}]\n");
    let path = "permissions/default.toml";
    if std::fs::read_to_string(path).ok().as_deref() != Some(default.as_str()) {
        std::fs::create_dir_all("permissions").expect("create permissions");
        std::fs::write(path, default).expect("write permissions/default.toml");
    }
    tauri_plugin::Builder::new(COMMANDS).build();
}
