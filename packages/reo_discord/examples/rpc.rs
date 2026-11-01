use std::time::Duration;

use reo_common::domain::{ContentType, GameLoader};
use reo_discord::{DiscordRpc, GameSession, LauncherActivity, PresenceOptions};

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let rpc = DiscordRpc::spawn(true, PresenceOptions::default());

    println!("launcher home for 10s...");
    std::thread::sleep(Duration::from_secs(10));

    println!("browsing mods for 10s...");
    rpc.set_activity(LauncherActivity::Browsing(ContentType::Mod));
    std::thread::sleep(Duration::from_secs(10));

    println!("playing for 10s...");
    rpc.set_game(Some(GameSession {
        cluster: "Example Cluster".to_owned(),
        mc_version: "1.21.11".to_owned(),
        loader: GameLoader::Fabric,
        mods: Some(42),
    }));
    std::thread::sleep(Duration::from_secs(10));

    println!("playing with details hidden for 10s...");
    rpc.set_options(PresenceOptions {
        show_game_details: false,
        show_launcher_activity: true,
    });
    std::thread::sleep(Duration::from_secs(10));

    println!("back in the launcher for 10s...");
    rpc.set_game(None);
    std::thread::sleep(Duration::from_secs(10));

    println!("shutting down");
    rpc.shutdown();
    std::thread::sleep(Duration::from_secs(1));
}
