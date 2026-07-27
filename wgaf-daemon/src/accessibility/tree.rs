//! Accessible tree walking and single-element info extraction, shared by
//! `ListApps`/`FindElements`/`GetTree`/`GetElementInfo`.
//!
//! Traversal is iterative (an explicit stack), not recursive `async fn` —
//! Rust doesn't support recursive `async fn` without boxing each call
//! (`Box::pin`), and an explicit stack is simpler here anyway since it also
//! gives depth-first pre-order for free (the order `wgaf a11y tree` renders
//! in) and a natural place to enforce the node-count safety cap (see
//! `mod.rs`'s `MAX_TREE_NODES`).

use atspi::ObjectRefOwned;
use atspi::proxy::accessible::AccessibleProxy;
use wgaf_common::{ElementRecord, ElementRef, TreeNode};

use super::{AccessibilityError, translate_element_error};

/// Converts an AT-SPI [`ObjectRefOwned`] (the `(so)` bus-name/object-path
/// pair every `Accessible`/`Collection` method deals in) into our own
/// [`ElementRef`] DTO. Returns `None` for a `Null` object reference — AT-SPI
/// uses these for e.g. "no parent"/an out-of-range child index in some
/// toolkits (see `AccessibleProxy::get_child_at_index`'s docs) — callers
/// should simply skip these rather than treat them as an error.
pub(crate) fn element_ref_from_object_ref(obj: &ObjectRefOwned) -> Option<ElementRef> {
    Some(ElementRef {
        bus_name: obj.name_as_str()?.to_string(),
        object_path: obj.path_as_str().to_string(),
    })
}

/// Reads the `Name`/`Description`/`Role`/`State`/`ChildCount` fields for one
/// accessible object and assembles them into an [`ElementRecord`]. Every
/// field read propagates its (translated) error as-is (no
/// `unwrap_or_default`) — silently defaulting a failed read (e.g. because
/// the element just went stale) would misreport a real error as "an element
/// with an empty name", which `translate_element_error` needs to see the
/// underlying `zbus::Error` for in order to report `ElementNotFound`
/// correctly.
///
/// **`role` is `GetRoleName`'s string, not `GetRole`'s fixed numeric-enum
/// name (`atspi::Role::name()`)** — this was the original design (reasoned
/// as "stable, locale-independent"), but real-`gtk4-demo`
/// integration testing found it actively misleading for modern GTK4
/// widgets: a `GtkSearchEntry` reports numeric role 87, whose *fixed*
/// AT-SPI enum name is `"form"` (the closest slot the decades-old
/// enumeration has), while its own `GetRoleName()` correctly/specifically
/// returns `"search"`. Likewise a top-level `GtkWindow` is numeric role 23
/// (`atspi::Role::Frame`, canonical name `"frame"`), while `GetRoleName()`
/// returns the friendlier `"window"`. `GetRoleName()` is also what
/// AT-SPI's own docs recommend for exactly this situation (see
/// `atspi::proxy::accessible::AccessibleProxy::get_role_name`'s docs: "will
/// return useful values for roles that fall outside the enumeration") and
/// what real AT tooling (Orca, Accerciser) surfaces to users — so
/// `--role` filtering matches what a user inspecting the app with those
/// tools would actually see, at the cost of technically being a
/// locale-dependent string (acceptable: this project automates a specific
/// user's own session, not a locale-agnostic script run on unknown
/// machines).
pub(crate) async fn element_record(
    proxy: &AccessibleProxy<'_>,
    element: ElementRef,
) -> Result<ElementRecord, AccessibilityError> {
    let name = proxy.name().await.map_err(translate_element_error)?;
    // See this function's doc comment for why `GetRoleName`, not `GetRole`.
    let role = proxy
        .get_role_name()
        .await
        .map_err(translate_element_error)?;
    let description = proxy.description().await.map_err(translate_element_error)?;
    let states = proxy.get_state().await.map_err(translate_element_error)?;
    let child_count = proxy.child_count().await.map_err(translate_element_error)?;
    Ok(ElementRecord {
        element,
        name,
        role,
        description,
        child_count,
        states: states.iter().map(|s| format!("{s:?}")).collect(),
    })
}

/// Depth-first, pre-order walk of the accessible tree rooted at `root`,
/// flattened into a `Vec<TreeNode>` with each node's `depth` relative to
/// `root` (which is `depth = 0`). Descent stops past `max_depth` (children
/// are not fetched for a node at `max_depth`) and the walk stops early once
/// `max_nodes` nodes have been collected (a safety cap — some accessible
/// trees, e.g. a browser's or an Electron app's, can be enormous, and this
/// guards against a single `FindElements`/`GetTree` call effectively
/// hanging or exhausting memory).
pub(crate) async fn walk(
    conn: &zbus::Connection,
    root: ElementRef,
    max_depth: u32,
    max_nodes: usize,
) -> Result<Vec<TreeNode>, AccessibilityError> {
    let mut result = Vec::new();
    let mut stack = vec![(root, 0u32)];

    while let Some((element, depth)) = stack.pop() {
        if result.len() >= max_nodes {
            tracing::warn!(
                target: super::AUDIT_TARGET,
                max_nodes,
                "accessible tree walk hit the node-count safety cap; results are truncated"
            );
            break;
        }

        let proxy = super::accessible_proxy_for(conn, &element).await?;
        let record = element_record(&proxy, element).await?;
        let children = if depth < max_depth {
            proxy
                .get_children()
                .await
                .map_err(translate_element_error)?
        } else {
            Vec::new()
        };

        result.push(TreeNode {
            element: record.element,
            name: record.name,
            role: record.role,
            description: record.description,
            child_count: record.child_count,
            states: record.states,
            depth,
        });

        // Push in reverse so children pop (and thus get visited) in their
        // original left-to-right order.
        for child in children.into_iter().rev() {
            if let Some(child_ref) = element_ref_from_object_ref(&child) {
                stack.push((child_ref, depth + 1));
            }
        }
    }

    Ok(result)
}

/// Drops a [`TreeNode`]'s `depth` field to get a plain [`ElementRecord`] —
/// used by `FindElements`, which walks the tree via [`walk`] but reports
/// results without depth information (that's a `GetTree`-only concept).
pub(crate) fn to_element_record(node: TreeNode) -> ElementRecord {
    ElementRecord {
        element: node.element,
        name: node.name,
        role: node.role,
        description: node.description,
        child_count: node.child_count,
        states: node.states,
    }
}
