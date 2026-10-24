pub const WINDOW_APP_ID: &str = "reo_app";
pub const WINDOW_TITLE: &str = "Reo Launcher";

// Public half of the key the release workflow signs updates with (the
// CARGO_PACKAGER_SIGN_PRIVATE_KEY secret); key id FE6F1C9F672EA798
pub const UPDATER_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEZFNkYxQzlGNjcyRUE3OTgKUldTWXB5NW5ueHh2L29Oait4ZFl5Y3NGNWwyY05JZTNxcXNtMGMzbDlNRDR1OHFsZnVYUXNyMEYK";
pub const UPDATER_ENDPOINT: &str =
    "https://github.com/clarkshusband/reo-launcher/releases/latest/download/latest.json";
pub const RELEASES_URL: &str = "https://github.com/clarkshusband/reo-launcher/releases/latest";
