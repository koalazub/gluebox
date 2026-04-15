pub struct ToolEntry {
    pub name: &'static str,
    pub description: &'static str,
    pub example: &'static str,
}

pub fn tool_registry() -> Vec<ToolEntry> {
    vec![
        ToolEntry {
            name: "samaya.listen",
            description: "Start real-time speech transcription",
            example: "samaya listen",
        },
        ToolEntry {
            name: "samaya.transcribe",
            description: "Transcribe an audio file",
            example: "samaya transcribe path/to/audio.wav",
        },
        ToolEntry {
            name: "samaya.history",
            description: "Show transcription history",
            example: "samaya history",
        },
        ToolEntry {
            name: "samaya.watch",
            description: "Watch a directory for new audio files to transcribe",
            example: "samaya watch path/to/dir",
        },
        ToolEntry {
            name: "samaya.devices",
            description: "List available audio input devices",
            example: "samaya devices",
        },
        ToolEntry {
            name: "samaya.doctor",
            description: "Check samaya installation and configuration",
            example: "samaya doctor",
        },
        ToolEntry {
            name: "sayl.health",
            description: "Check sayl service health",
            example: "sayl health",
        },
        ToolEntry {
            name: "sayl.models",
            description: "List available sayl models",
            example: "sayl models",
        },
        ToolEntry {
            name: "sayl.pull",
            description: "Pull a sayl model",
            example: "sayl pull <model>",
        },
        ToolEntry {
            name: "sayl.doctor",
            description: "Check sayl installation and configuration",
            example: "sayl doctor",
        },
        ToolEntry {
            name: "apfel",
            description: "Run apfel AI assistant",
            example: "apfel",
        },
        ToolEntry {
            name: "apfel.chat",
            description: "Start an apfel chat session",
            example: "apfel chat",
        },
        ToolEntry {
            name: "gluebox.status",
            description: "Show gluebox daemon status",
            example: "gluebox status",
        },
        ToolEntry {
            name: "gluebox.import",
            description: "Import a session into gluebox",
            example: "gluebox import <session_id>",
        },
        ToolEntry {
            name: "gluebox.study",
            description: "Run gluebox study mode",
            example: "gluebox study",
        },
        ToolEntry {
            name: "gluebox.toggle",
            description: "Toggle a gluebox connector",
            example: "gluebox toggle <connector>",
        },
    ]
}

pub static ALLOWED_BINARIES: &[&str] = &["samaya", "sayl", "apfel", "gluebox"];

pub fn is_command_allowed(command: &str) -> bool {
    let first_word = command.split_whitespace().next().unwrap_or("");
    // Only the basename matters — strip any path prefix
    let binary = std::path::Path::new(first_word)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(first_word);
    ALLOWED_BINARIES.contains(&binary)
}
