//! Looking for a newer release of this application on GitHub.
//!
//! The application asks the GitHub API for the latest release of its own
//! repository; the core compares the versions. The request carries nothing but
//! a fixed client name. Installing is left to the user: the release page opens
//! in the browser.

/// Version of this build. CI sets `BTM_VERSION` to the version it releases.
pub const VERSION: &str = match option_env!("BTM_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// "owner/repository" whose releases are checked. CI sets it to the
/// repository it builds.
const REPOSITORY: &str = match option_env!("BTM_UPDATE_REPOSITORY") {
    Some(repository) => repository,
    None => "fhnb16/btManage-linux",
};

/// Part of the name of the release file for this processor.
const ASSET_HINT: &str = if cfg!(target_arch = "aarch64") {
    "linux-aarch64"
} else {
    "linux-x86_64"
};

pub struct Found {
    pub version: String,
    /// The release page.
    pub url: String,
}

/// Asks GitHub for the latest release. `None` when there is nothing newer or
/// GitHub cannot be reached. Blocks: call it off the UI thread.
pub fn check() -> Option<Found> {
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let release = ureq::get(&url)
        .header(
            "User-Agent",
            concat!("btManage/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let found = btmanage_core::update::check(VERSION, &release, ASSET_HINT)?;
    Some(Found {
        version: found.version,
        url: found.page_url,
    })
}
