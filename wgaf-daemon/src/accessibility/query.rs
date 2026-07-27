//! Pure element/application filtering logic for `FindElements`. Deliberately
//! has zero AT-SPI/D-Bus dependency — everything here operates on plain
//! strings and [`wgaf_common::AppRecord`], so it's unit-testable without a
//! live accessibility bus (see this module's tests), unlike the rest of
//! `accessibility/`.

use wgaf_common::AppRecord;

/// Finds the application whose name best matches `app_name`: an exact,
/// case-insensitive match wins first; failing that, the first
/// case-insensitive substring match. Ambiguity (multiple substring matches)
/// is deliberately resolved by "first match wins", not an error — this
/// mirrors the project's existing preference for simple, predictable
/// behavior over a more elaborate ranking scheme, given `wgaf a11y list-apps`
/// already lets a caller disambiguate manually if needed.
pub(crate) fn find_matching_app<'a>(
    apps: &'a [AppRecord],
    app_name: &str,
) -> Option<&'a AppRecord> {
    apps.iter()
        .find(|a| a.name.eq_ignore_ascii_case(app_name))
        .or_else(|| {
            let needle = app_name.to_lowercase();
            apps.iter()
                .find(|a| a.name.to_lowercase().contains(&needle))
        })
}

/// Whether one element's `role`/`name`/`description` satisfy the given
/// filters. Each filter is ignored (treated as "matches anything") when
/// empty — `role` is matched case-insensitively as a whole (it's a short,
/// enumerable-in-practice word/phrase — see `tree::element_record`'s doc
/// comment for why `role` is `GetRoleName`'s string rather than a fixed
/// enum name), while `name`/`description` are case-insensitive substring
/// matches (free-form text, e.g. a button labelled "Save As...' should
/// still be found by
/// `--name save`).
pub(crate) fn matches(
    role: &str,
    name: &str,
    description: &str,
    role_filter: &str,
    name_filter: &str,
    description_filter: &str,
) -> bool {
    (role_filter.is_empty() || role.eq_ignore_ascii_case(role_filter))
        && (name_filter.is_empty() || name.to_lowercase().contains(&name_filter.to_lowercase()))
        && (description_filter.is_empty()
            || description
                .to_lowercase()
                .contains(&description_filter.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str) -> AppRecord {
        AppRecord {
            element: wgaf_common::ElementRef {
                bus_name: ":1.1".to_string(),
                object_path: "/org/a11y/atspi/accessible/root".to_string(),
            },
            name: name.to_string(),
        }
    }

    #[test]
    fn find_matching_app_prefers_exact_case_insensitive_match() {
        let apps = vec![app("gtk4-demo"), app("GTK4 Demo Widget Factory")];
        let found = find_matching_app(&apps, "GTK4-DEMO").expect("should match");
        assert_eq!(found.name, "gtk4-demo");
    }

    #[test]
    fn find_matching_app_falls_back_to_substring_match() {
        let apps = vec![app("gtk4-widget-factory")];
        let found = find_matching_app(&apps, "widget").expect("should match");
        assert_eq!(found.name, "gtk4-widget-factory");
    }

    #[test]
    fn find_matching_app_returns_none_when_nothing_matches() {
        let apps = vec![app("gtk4-demo")];
        assert!(find_matching_app(&apps, "nonexistent").is_none());
    }

    #[test]
    fn matches_empty_filters_match_anything() {
        assert!(matches("push button", "Save", "Saves the file", "", "", ""));
    }

    #[test]
    fn matches_role_filter_is_case_insensitive_and_whole_string() {
        assert!(matches("push button", "Save", "", "Push Button", "", ""));
        assert!(!matches("push button", "Save", "", "button", "", ""));
    }

    #[test]
    fn matches_name_filter_is_case_insensitive_substring() {
        assert!(matches("push button", "Save As...", "", "", "save", ""));
        assert!(!matches("push button", "Cancel", "", "", "save", ""));
    }

    #[test]
    fn matches_description_filter_is_case_insensitive_substring() {
        assert!(matches(
            "push button",
            "Save",
            "Saves the current document",
            "",
            "",
            "current document"
        ));
        assert!(!matches(
            "push button",
            "Save",
            "Saves the current document",
            "",
            "",
            "nonexistent"
        ));
    }

    #[test]
    fn matches_requires_all_given_filters_to_pass() {
        assert!(!matches(
            "push button",
            "Save",
            "",
            "menu item", // wrong role
            "save",
            ""
        ));
    }
}
