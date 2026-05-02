use zed_extension_api as zed;

struct LocalHistoryExtension;

impl zed::Extension for LocalHistoryExtension {
    fn new() -> Self {
        Self
    }

    fn run_slash_command(
        &self,
        command: zed::SlashCommand,
        _args: Vec<String>,
        _worktree: Option<&zed::Worktree>,
    ) -> Result<zed::SlashCommandOutput, String> {
        let text = match command.name.as_str() {
            "local-history-status" => {
                "local-history Zed integration scaffold is installed. Sidecar bootstrap logic is planned but not implemented yet."
            }
            "local-history-recent" => {
                "Recent snapshot browsing will be provided by the native sidecar and exposed through Zed-supported surfaces."
            }
            "local-history-view" => {
                "Markdown history view generation belongs to the native sidecar. This extension will later locate or reveal those files."
            }
            _ => "Unknown local-history slash command.",
        };

        Ok(zed::SlashCommandOutput {
            text: text.to_string(),
            sections: Vec::new(),
        })
    }
}

zed::register_extension!(LocalHistoryExtension);
