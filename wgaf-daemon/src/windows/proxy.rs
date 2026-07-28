//! `zbus` client proxy for the GNOME Shell Extension's window-management
//! D-Bus interface. This must match `extension/dbusInterface.js`'s
//! `DBUS_INTERFACE_XML` exactly — method names/arities/types are checked at
//! runtime by `zbus`, not the compiler.
//!
//! The `tests` module below closes that gap: it parses the extension's
//! interface XML at compile time and asserts every method and signal matches
//! this file, so a divergence fails `cargo test` instead of surfacing as a
//! runtime `UnknownMethod` on a user's desktop.
//!
//! The `interface`/`default_service`/`default_path` attributes below must
//! be string literals (macro limitation), so they can't reference
//! `wgaf_common::EXTENSION_*` directly; a unit test in this module asserts
//! they stay in sync instead. `WindowManager::connect_to` (see `mod.rs`)
//! additionally allows overriding the destination/path at runtime, which
//! `wgaf_common::EXTENSION_BUS_NAME`/`EXTENSION_OBJECT_PATH` are passed
//! into in production — the attributes below only supply the fallback
//! defaults used by `ShellExtensionProxy::new`.

use wgaf_common::dict::{WindowRecordDict, WorkspaceRecordDict};

#[zbus::proxy(
    interface = "org.gnome.Shell.Extensions.Wgaf.V1",
    default_service = "org.gnome.Shell.Extensions.Wgaf",
    default_path = "/org/gnome/Shell/Extensions/Wgaf"
)]
pub(crate) trait ShellExtension {
    /// Enumerate all windows known to Mutter.
    fn list_windows(&self) -> zbus::Result<Vec<WindowRecordDict>>;

    /// Focus (activate) the window with the given id.
    fn focus_window(&self, id: u32) -> zbus::Result<()>;

    /// Move the window with the given id so its top-left corner is at
    /// `(x, y)`.
    fn move_window(&self, id: u32, x: i32, y: i32) -> zbus::Result<()>;

    /// Resize the window with the given id to `(width, height)`.
    fn resize_window(&self, id: u32, width: i32, height: i32) -> zbus::Result<()>;

    /// Close (request deletion of) the window with the given id.
    fn close_window(&self, id: u32) -> zbus::Result<()>;

    /// Enumerate all workspaces.
    fn get_workspaces(&self) -> zbus::Result<Vec<WorkspaceRecordDict>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use zbus::zvariant::Type;

    /// The extension's D-Bus contract, read at compile time. `include_str!`
    /// resolves relative to this file; the path is stable because both live in
    /// the same repository and ship together.
    ///
    /// Test-only, so the daemon build does not depend on the extension source.
    const EXTENSION_SOURCE: &str = include_str!("../../../extension/dbusInterface.js");

    /// One method or signal's argument types, as D-Bus signature strings.
    ///
    /// Signals have no `in` arguments; their payload lives in `out`, matching
    /// how D-Bus itself treats signal arguments as outbound.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct Args {
        input: Vec<String>,
        output: Vec<String>,
    }

    impl Args {
        fn new(input: Vec<String>, output: Vec<String>) -> Self {
            Args { input, output }
        }
    }

    /// The D-Bus signature of a Rust type, e.g. `u32` → `"u"`,
    /// `Vec<WindowRecordDict>` → `"aa{sv}"`.
    ///
    /// The expectation table names Rust types rather than signature strings
    /// so that it reads as the trait does, and so a wire-format change to
    /// `WindowRecordDict` (which owns its own `a{sv}` signature attribute)
    /// flows through without anyone editing a string here.
    fn sig<T: Type>() -> String {
        T::SIGNATURE.to_string()
    }

    /// Value of a single-quoted JS `const` in [`EXTENSION_SOURCE`], e.g.
    /// `DBUS_BUS_NAME` → `"org.gnome.Shell.Extensions.Wgaf"`.
    fn js_const(name: &str) -> String {
        let needle = format!("const {name} = '");
        let start = EXTENSION_SOURCE
            .find(&needle)
            .unwrap_or_else(|| panic!("no `const {name} = '...'` in dbusInterface.js"))
            + needle.len();
        let rest = &EXTENSION_SOURCE[start..];
        let end = rest
            .find('\'')
            .unwrap_or_else(|| panic!("unterminated string for const {name}"));
        rest[..end].to_string()
    }

    /// Substitute `${SOME_CONST}` template-literal interpolations with the
    /// value of the corresponding JS `const`. The extension builds both its
    /// interface XML and its error names this way.
    fn resolve_interpolations(s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(open) = rest.find("${") {
            out.push_str(&rest[..open]);
            let tail = &rest[open + 2..];
            let close = tail.find('}').expect("unterminated ${...} interpolation");
            out.push_str(&js_const(&tail[..close]));
            rest = &tail[close + 1..];
        }
        out.push_str(rest);
        out
    }

    /// Body of a backtick-delimited template literal assigned to `name`,
    /// with interpolations resolved.
    fn js_template_literal(name: &str) -> String {
        let needle = format!("{name} = `");
        let start = EXTENSION_SOURCE
            .find(&needle)
            .unwrap_or_else(|| panic!("no ``{name} = `...` `` in dbusInterface.js"))
            + needle.len();
        let rest = &EXTENSION_SOURCE[start..];
        let end = rest
            .find('`')
            .unwrap_or_else(|| panic!("unterminated template literal for {name}"));
        resolve_interpolations(&rest[..end])
    }

    /// Value of an XML attribute in a single tag's text.
    fn attr<'a>(tag: &'a str, key: &str) -> Option<&'a str> {
        let needle = format!("{key}=\"");
        let start = tag.find(&needle)? + needle.len();
        let rest = &tag[start..];
        rest.find('"').map(|end| &rest[..end])
    }

    #[derive(Debug, Default)]
    struct InterfaceXml {
        name: String,
        methods: BTreeMap<String, Args>,
        signals: BTreeMap<String, Args>,
    }

    /// Minimal scanner for the subset of D-Bus introspection XML the extension
    /// emits: one `<interface>` containing `<method>`/`<signal>` elements of
    /// `<arg>`s. Hand-rolled rather than pulling in an XML crate for a single
    /// test over a file this project controls end to end.
    fn parse_interface_xml(xml: &str) -> InterfaceXml {
        enum Current {
            None,
            Method(String, Args),
            Signal(String, Args),
        }

        let mut parsed = InterfaceXml::default();
        let mut current = Current::None;

        for raw in xml.split('<').skip(1) {
            let tag = raw
                .split('>')
                .next()
                .expect("split always yields one element")
                .trim()
                .trim_end_matches('/')
                .trim();
            let kind = tag.split_whitespace().next().unwrap_or("");
            let name = || {
                attr(tag, "name")
                    .unwrap_or_else(|| panic!("<{kind}> without a name attribute"))
                    .to_string()
            };

            match kind {
                "interface" => parsed.name = name(),
                "method" => current = Current::Method(name(), Args::default()),
                "signal" => current = Current::Signal(name(), Args::default()),
                "arg" => {
                    let ty = attr(tag, "type")
                        .expect("<arg> without a type attribute")
                        .to_string();
                    match &mut current {
                        Current::Method(_, args) => {
                            if attr(tag, "direction") == Some("in") {
                                args.input.push(ty);
                            } else {
                                args.output.push(ty);
                            }
                        }
                        Current::Signal(_, args) => args.output.push(ty),
                        Current::None => panic!("<arg> outside a method or signal"),
                    }
                }
                "/method" => {
                    if let Current::Method(name, args) =
                        std::mem::replace(&mut current, Current::None)
                    {
                        parsed.methods.insert(name, args);
                    }
                }
                "/signal" => {
                    if let Current::Signal(name, args) =
                        std::mem::replace(&mut current, Current::None)
                    {
                        parsed.signals.insert(name, args);
                    }
                }
                _ => {}
            }
        }

        parsed
    }

    fn extension_interface() -> InterfaceXml {
        parse_interface_xml(&js_template_literal("DBUS_INTERFACE_XML"))
    }

    #[test]
    fn proxy_attributes_match_wgaf_common_constants() {
        assert_eq!(
            wgaf_common::EXTENSION_INTERFACE_NAME,
            "org.gnome.Shell.Extensions.Wgaf.V1"
        );
        assert_eq!(
            wgaf_common::EXTENSION_BUS_NAME,
            "org.gnome.Shell.Extensions.Wgaf"
        );
        assert_eq!(
            wgaf_common::EXTENSION_OBJECT_PATH,
            "/org/gnome/Shell/Extensions/Wgaf"
        );
    }

    /// The names the extension actually exports must be the ones the daemon
    /// dials. Renaming either side alone leaves the daemon reporting
    /// "extension not available" against an extension that is running fine.
    #[test]
    fn extension_constants_match_wgaf_common_constants() {
        assert_eq!(js_const("DBUS_BUS_NAME"), wgaf_common::EXTENSION_BUS_NAME);
        assert_eq!(
            js_const("DBUS_OBJECT_PATH"),
            wgaf_common::EXTENSION_OBJECT_PATH
        );
        assert_eq!(
            js_const("DBUS_INTERFACE_NAME"),
            wgaf_common::EXTENSION_INTERFACE_NAME
        );
        assert_eq!(
            extension_interface().name,
            wgaf_common::EXTENSION_INTERFACE_NAME,
            "the interface XML declares a different name than DBUS_INTERFACE_NAME"
        );
    }

    /// `translate_window_error` matches on this name to turn the extension's
    /// failure into `org.wgaf.Windows1.Error.WindowNotFound`; a rename on the
    /// extension side would silently degrade it to a generic error.
    #[test]
    fn extension_error_name_matches_wgaf_common_constant() {
        let start = EXTENSION_SOURCE
            .find("WINDOW_NOT_FOUND: `")
            .expect("no WINDOW_NOT_FOUND entry in DBusErrors")
            + "WINDOW_NOT_FOUND: `".len();
        let rest = &EXTENSION_SOURCE[start..];
        let end = rest
            .find('`')
            .expect("unterminated WINDOW_NOT_FOUND literal");

        assert_eq!(
            resolve_interpolations(&rest[..end]),
            wgaf_common::EXTENSION_ERROR_WINDOW_NOT_FOUND
        );
    }

    /// Every method on [`ShellExtensionProxy`] must exist in the extension's
    /// interface XML with the same argument types, and vice versa.
    ///
    /// The expectation table below transcribes the trait by hand: `zbus`
    /// exposes no compile-time metadata about a proxy, so nothing can derive
    /// it. That bounds what this test is for. It catches the **extension**
    /// changing out from under the daemon — a renamed method, a changed
    /// argument type, a new method the daemon has no proxy for — which is the
    /// direction nothing else covers, since the extension is a separate
    /// GJS program the compiler never sees.
    ///
    /// The reverse direction is already covered elsewhere and deliberately
    /// not duplicated here: changing an argument's type in the trait fails to
    /// compile against its callers in `mod.rs`, and `tests/windows_stub.rs`
    /// exercises the real dispatch against a stub extension. What is left
    /// uncovered is adding a method to *both* the trait and the extension and
    /// forgetting this table — harmless, since both real sides agree.
    #[test]
    fn proxy_methods_match_extension_interface_xml() {
        let (u, i) = (sig::<u32>(), sig::<i32>());
        let expected: BTreeMap<String, Args> = [
            (
                "ListWindows",
                Args::new(vec![], vec![sig::<Vec<WindowRecordDict>>()]),
            ),
            ("FocusWindow", Args::new(vec![u.clone()], vec![])),
            (
                "MoveWindow",
                Args::new(vec![u.clone(), i.clone(), i.clone()], vec![]),
            ),
            (
                "ResizeWindow",
                Args::new(vec![u.clone(), i.clone(), i], vec![]),
            ),
            ("CloseWindow", Args::new(vec![u], vec![])),
            (
                "GetWorkspaces",
                Args::new(vec![], vec![sig::<Vec<WorkspaceRecordDict>>()]),
            ),
        ]
        .into_iter()
        .map(|(name, args)| (name.to_string(), args))
        .collect();

        assert_eq!(
            extension_interface().methods,
            expected,
            "proxy.rs and extension/dbusInterface.js disagree about the \
             methods on {}",
            wgaf_common::EXTENSION_INTERFACE_NAME
        );
    }

    /// The extension's signals are pinned even though the proxy does not yet
    /// subscribe to them, so their shape cannot drift out from under the
    /// subscription work before it lands. Add matching `#[zbus(signal)]`
    /// declarations to the trait above when it does.
    #[test]
    fn extension_signals_match_the_expected_shapes() {
        let u = sig::<u32>();
        let expected: BTreeMap<String, Args> = [
            (
                "WindowCreated",
                Args::new(vec![], vec![sig::<WindowRecordDict>()]),
            ),
            ("WindowClosed", Args::new(vec![], vec![u.clone()])),
            ("WindowFocusChanged", Args::new(vec![], vec![u])),
        ]
        .into_iter()
        .map(|(name, args)| (name.to_string(), args))
        .collect();

        assert_eq!(
            extension_interface().signals,
            expected,
            "the signals on {} changed shape",
            wgaf_common::EXTENSION_INTERFACE_NAME
        );
    }
}
