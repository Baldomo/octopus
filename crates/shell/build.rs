macro_rules! plugin_commands {
    ($($command:ident),* $(,)?) => {
        /// Every command of the `octopus` plugin, each granted by its `default` permission.
        const COMMANDS: &[&str] = &[$(stringify!($command)),*];
    };
}

include!("plugin_commands.rs");

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
