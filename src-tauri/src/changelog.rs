//! The changelog, compiled in.
//!
//! Read from the repository at build time rather than fetched at runtime: what a build
//! changed is a property of that build, and a release that has to reach the network to
//! explain itself cannot explain itself offline, or after the release notes are edited.
//!
//! Only the section for the running version is ever shown. Someone who has just been
//! updated wants to know what changed in the thing they are now running, not to read the
//! whole history of the project.

/// The whole file, as committed.
const CHANGELOG: &str = include_str!("../../CHANGELOG.md");

/// The section for a version, without its heading, or `None` if there is no such section.
///
/// Sections are `## <version>` and run until the next `##`. The build being unable to find
/// its own entry is not an error worth failing over: the UI simply has nothing to show, and
/// the release script is what makes sure the entry exists in the first place.
pub fn for_version(version: &str) -> Option<String> {
    let heading = format!("## {version}");

    let start = CHANGELOG
        .lines()
        .position(|line| line.trim() == heading)?;

    let body: Vec<&str> = CHANGELOG
        .lines()
        .skip(start + 1)
        .take_while(|line| !line.trim_start().starts_with("## "))
        .collect();

    let text = body.join("\n").trim().to_string();

    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_running_version_has_an_entry() {
        // The release script enforces this too, but a build whose own changelog is missing
        // should fail here, where it is cheap to notice.
        assert!(
            for_version(env!("CARGO_PKG_VERSION")).is_some(),
            "CHANGELOG.md has no `## {}` section",
            env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    fn a_section_stops_at_the_next_version() {
        let text = for_version("0.1.0").expect("0.1.0 should have an entry");
        assert!(text.contains("First build"));
        assert!(!text.contains("## "), "a section must not run into the next");
    }

    #[test]
    fn an_unknown_version_has_nothing_to_show() {
        assert_eq!(for_version("9.9.9"), None);
    }

    #[test]
    fn a_version_is_matched_whole() {
        // `## 0.1.0` must not be found by asking for `0.1`.
        assert_eq!(for_version("0.1"), None);
    }
}
