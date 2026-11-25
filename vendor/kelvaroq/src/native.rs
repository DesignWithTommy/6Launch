use anyhow::{Context, Result, bail};
use std::{
    fs::File,
    path::Path,
    process::{Command, ExitCode},
};
use windows::{
    Win32::System::Console::{AllocConsole, SetConsoleTitleW},
    core::w,
};

// A process-local test profile. An explicit per-feature setting always wins.
const EXPERIMENTAL_RENDER: &[(&str, &str)] = &[
    ("VICTORY_GPU_UPLOAD_READONLY_MB", "256"),
    ("VICTORY_CARD_READONLY_MB", "128"),
    ("VICTORY_RECYCLE_COMPARE_SLOTS", "1"),
    ("VICTORY_GPU_CONSTANT_FILLS", "1"),
    ("VICTORY_TEXTURE_NO_WAIT", "1"),
    ("VICTORY_TEXTURE_SCOPED_REMAP", "1"),
    ("VICTORY_DESCRIPTOR_SYNC_BATCH", "1"),
    ("VICTORY_ROOT_BIND_CACHE", "1"),
    ("VICTORY_RELAY_COPY_RUNS", "1"),
    ("VICTORY_PIPELINE_EARLY_WITHOUT_LIBRARY", "1"),
    ("VICTORY_SCOPED_CARD_BARRIERS", "1"),
];

pub(crate) fn run(
    engine: &Path,
    image: &Path,
    console: &str,
    experimental_render: bool,
) -> Result<ExitCode> {
    // Give the native file services a conventional DOS path when it names
    // the same file. Keep the extended form for paths that require it.
    let image = dunce::canonicalize(image).context("ouverture de l’exécutable du jeu")?;
    let configure = || -> Result<Command> {
        let mut game = Command::new(engine);
        if experimental_render {
            game.envs(
                EXPERIMENTAL_RENDER
                    .iter()
                    .copied()
                    .filter(|(key, _)| std::env::var_os(key).is_none()),
            );
        }
        game.current_dir(engine.parent().context("dossier du moteur natif")?)
            .env(
                "VICTORY_GAME_DIR",
                image.parent().context("dossier du jeu")?,
            );
        Ok(game)
    };
    if std::env::var_os("KELVAROQ_DESKTOP_LOG").is_some()
        && let Err(error) = crate::preload_process::before_play(&configure)
    {
        eprintln!("Préchargement interrompu. {error:#}");
    }
    let mut game = configure()?;
    game.arg("run")
        .arg(&image)
        .args(["--console", console, "--execute"]);
    if std::env::var_os("KELVAROQ_DESKTOP_LOG").is_some() {
        // The desktop launcher starts this CLI without a console and captures its
        // stderr for the error dialog. Give the child its own visible log output.
        unsafe {
            AllocConsole().context("ouverture du journal Windows")?;
            SetConsoleTitleW(w!("Kelvaroq | Journal du jeu"))?;
        }
        let log = File::options().write(true).open("CONOUT$")?;
        game.stdout(log.try_clone()?).stderr(log);
    }
    let status = game
        .status()
        .with_context(|| format!("démarrage de {}", engine.display()))?;
    if !status.success() {
        bail!("Le moteur natif s’est arrêté ({status}).");
    }
    Ok(ExitCode::SUCCESS)
}
