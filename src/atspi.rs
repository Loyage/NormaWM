//! AT-SPI2 based UI composition queries.
//!
//! This module intentionally stays on the control-plane side of the compositor:
//! it reads accessibility metadata exposed by client applications over AT-SPI2
//! and does not participate in rendering or Wayland surface management.

use std::collections::BTreeMap;

use ::atspi::{
    connection::P2P,
    proxy::{accessible::AccessibleProxy, proxy_ext::ProxyExt},
    AccessibilityConnection, CoordType, Interface, ObjectRefOwned,
};
use futures_lite::future;
use serde::Serialize;

const MAX_APPLICATIONS: usize = 32;
const MAX_TREE_DEPTH: usize = 12;
const MAX_TREE_NODES: usize = 800;

#[derive(Debug, Clone, Serialize)]
pub struct WindowAccessibilityTarget {
    pub id: String,
    pub workspace: u8,
    pub title: Option<String>,
    pub app_id: Option<String>,
    pub focused: bool,
    pub human_control: bool,
    pub visible: bool,
    pub layout_geometry: JsonRectI32,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct JsonRectI32 {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowAccessibilityComposition {
    pub window: WindowAccessibilityTarget,
    pub accessibility: AtspiComposition,
}

#[derive(Debug, Clone, Serialize)]
pub struct AtspiComposition {
    pub protocol: &'static str,
    pub matched_by: String,
    pub applications_seen: Vec<AtspiApplicationSummary>,
    pub node_count: usize,
    pub truncated: bool,
    pub tree: AtspiNode,
}

#[derive(Debug, Clone, Serialize)]
pub struct AtspiApplicationSummary {
    pub bus_name: Option<String>,
    pub path: String,
    pub name: Option<String>,
    pub role: Option<String>,
    pub child_count: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AtspiNode {
    pub bus_name: Option<String>,
    pub path: String,
    pub depth: usize,
    pub name: Option<String>,
    pub role: Option<String>,
    pub role_debug: Option<String>,
    pub description: Option<String>,
    pub child_count: Option<i32>,
    pub interfaces: Vec<String>,
    pub attributes: BTreeMap<String, String>,
    pub component: Option<AtspiComponentInfo>,
    pub children: Vec<AtspiNode>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AtspiComponentInfo {
    pub screen_extents: JsonRectI32,
    pub window_extents: JsonRectI32,
    pub alpha: Option<f64>,
    pub layer: Option<String>,
    pub mdi_z_order: Option<i16>,
}

pub fn query_window_accessibility_tree(
    target: WindowAccessibilityTarget,
) -> Result<WindowAccessibilityComposition, String> {
    let connection = future::block_on(AccessibilityConnection::new())
        .map_err(|error| format!("failed to connect to AT-SPI2 registry: {error}"))?;
    let registry_root = future::block_on(connection.root_accessible_on_registry())
        .map_err(|error| format!("failed to open AT-SPI2 registry root: {error}"))?;
    let app_refs = future::block_on(registry_root.get_children())
        .map_err(|error| format!("failed to list AT-SPI2 applications: {error}"))?;

    let mut applications_seen = Vec::new();
    let mut best_match = None::<MatchedTree>;

    for app_ref in app_refs.into_iter().take(MAX_APPLICATIONS) {
        if app_ref.is_null() {
            continue;
        }

        let Ok(app_proxy) = future::block_on(connection.object_as_accessible(&app_ref)) else {
            continue;
        };
        applications_seen.push(application_summary(&app_ref, &app_proxy));

        let mut budget = TreeBudget::new(MAX_TREE_NODES);
        let mut tree = collect_node(&connection, &app_ref, &app_proxy, 0, &mut budget);
        let match_score = score_tree(&tree, &target);

        if match_score.identity_matched {
            if let Some(node) = best_scored_node(&tree, &target) {
                tree = node;
            }
        }

        if match_score.identity_matched
            && best_match
                .as_ref()
                .is_none_or(|current| match_score.score > current.score)
        {
            best_match = Some(MatchedTree {
                score: match_score.score,
                matched_by: match_score.matched_by,
                node_count: budget.visited,
                truncated: budget.truncated,
                tree,
            });
        }
    }

    if applications_seen.is_empty() {
        return Err("AT-SPI2 registry did not expose any applications".to_string());
    }

    let Some(best_match) = best_match else {
        return Err(format!(
            "no AT-SPI2 accessible tree matched window {} title={} app_id={}",
            target.id,
            target.title.as_deref().unwrap_or("<unset>"),
            target.app_id.as_deref().unwrap_or("<unset>")
        ));
    };

    Ok(WindowAccessibilityComposition {
        window: target,
        accessibility: AtspiComposition {
            protocol: "at-spi2",
            matched_by: best_match.matched_by,
            applications_seen,
            node_count: best_match.node_count,
            truncated: best_match.truncated,
            tree: best_match.tree,
        },
    })
}

fn application_summary(
    object_ref: &ObjectRefOwned,
    proxy: &AccessibleProxy<'_>,
) -> AtspiApplicationSummary {
    AtspiApplicationSummary {
        bus_name: object_ref.name_as_str().map(ToOwned::to_owned),
        path: object_ref.path_as_str().to_string(),
        name: optional_string(future::block_on(proxy.name())),
        role: optional_string(future::block_on(proxy.get_role_name())),
        child_count: future::block_on(proxy.child_count()).ok(),
    }
}

fn collect_node(
    connection: &AccessibilityConnection,
    object_ref: &ObjectRefOwned,
    proxy: &AccessibleProxy<'_>,
    depth: usize,
    budget: &mut TreeBudget,
) -> AtspiNode {
    if !budget.enter() {
        return truncated_node(object_ref, depth);
    }

    let mut errors = Vec::new();
    let name = optional_string(future::block_on(proxy.name()));
    let role = optional_string(future::block_on(proxy.get_role_name()));
    let role_debug = future::block_on(proxy.get_role())
        .map(|role| format!("{role:?}"))
        .ok();
    let description = optional_string(future::block_on(proxy.description()));
    let child_count = future::block_on(proxy.child_count()).ok();
    let attributes = future::block_on(proxy.get_attributes())
        .map(BTreeMap::from_iter)
        .unwrap_or_default();
    let interfaces = future::block_on(proxy.get_interfaces())
        .map(|interfaces| {
            interfaces
                .iter()
                .map(|interface| format!("{interface:?}"))
                .collect()
        })
        .unwrap_or_default();
    let component = collect_component(proxy, &mut errors);

    let mut children = Vec::new();
    if depth < MAX_TREE_DEPTH {
        match future::block_on(proxy.get_children()) {
            Ok(child_refs) => {
                for child_ref in child_refs {
                    if child_ref.is_null() {
                        continue;
                    }
                    if budget.exhausted() {
                        budget.truncated = true;
                        break;
                    }
                    match future::block_on(connection.object_as_accessible(&child_ref)) {
                        Ok(child_proxy) => children.push(collect_node(
                            connection,
                            &child_ref,
                            &child_proxy,
                            depth + 1,
                            budget,
                        )),
                        Err(error) => {
                            errors.push(format!(
                                "failed to open child {}: {error}",
                                child_ref.path_as_str()
                            ));
                        }
                    }
                }
            }
            Err(error) => errors.push(format!("failed to list children: {error}")),
        }
    } else if child_count.unwrap_or_default() > 0 {
        budget.truncated = true;
    }

    AtspiNode {
        bus_name: object_ref.name_as_str().map(ToOwned::to_owned),
        path: object_ref.path_as_str().to_string(),
        depth,
        name,
        role,
        role_debug,
        description,
        child_count,
        interfaces,
        attributes,
        component,
        children,
        errors,
    }
}

fn collect_component(
    proxy: &AccessibleProxy<'_>,
    errors: &mut Vec<String>,
) -> Option<AtspiComponentInfo> {
    let interfaces = future::block_on(proxy.get_interfaces()).ok()?;
    if !interfaces.contains(Interface::Component) {
        return None;
    }

    let proxies = match future::block_on(proxy.proxies()) {
        Ok(proxies) => proxies,
        Err(error) => {
            errors.push(format!("failed to create AT-SPI2 proxy set: {error}"));
            return None;
        }
    };
    let component = match future::block_on(proxies.component()) {
        Ok(component) => component,
        Err(error) => {
            errors.push(format!("failed to create Component proxy: {error}"));
            return None;
        }
    };

    let screen_extents = future::block_on(component.get_extents(CoordType::Screen))
        .map(rect_from_extents)
        .ok()?;
    let window_extents = future::block_on(component.get_extents(CoordType::Window))
        .map(rect_from_extents)
        .unwrap_or(screen_extents);
    let alpha = future::block_on(component.get_alpha()).ok();
    let layer = future::block_on(component.get_layer())
        .map(|layer| format!("{layer:?}"))
        .ok();
    let mdi_z_order = future::block_on(component.get_mdiz_order()).ok();

    Some(AtspiComponentInfo {
        screen_extents,
        window_extents,
        alpha,
        layer,
        mdi_z_order,
    })
}

fn truncated_node(object_ref: &ObjectRefOwned, depth: usize) -> AtspiNode {
    AtspiNode {
        bus_name: object_ref.name_as_str().map(ToOwned::to_owned),
        path: object_ref.path_as_str().to_string(),
        depth,
        name: None,
        role: None,
        role_debug: None,
        description: None,
        child_count: None,
        interfaces: Vec::new(),
        attributes: BTreeMap::new(),
        component: None,
        children: Vec::new(),
        errors: vec!["AT-SPI2 tree output truncated".to_string()],
    }
}

fn score_tree(node: &AtspiNode, target: &WindowAccessibilityTarget) -> MatchScore {
    let mut best = score_node(node, target);
    for child in &node.children {
        let child_score = score_tree(child, target);
        if child_score.is_better_than(&best) {
            best = child_score;
        }
    }
    best
}

fn best_scored_node(node: &AtspiNode, target: &WindowAccessibilityTarget) -> Option<AtspiNode> {
    let mut best_score = score_node(node, target);
    let mut best_node = if best_score.identity_matched {
        Some(node.clone())
    } else {
        None
    };

    for child in &node.children {
        if let Some(child_node) = best_scored_node(child, target) {
            let child_score = score_node(&child_node, target);
            if child_score.is_better_than(&best_score) {
                best_score = child_score;
                best_node = Some(child_node);
            }
        }
    }

    best_node
}

fn score_node(node: &AtspiNode, target: &WindowAccessibilityTarget) -> MatchScore {
    let mut score = 0;
    let mut identity_matched = false;
    let mut matched_by = Vec::new();
    let name = node.name.as_deref().unwrap_or_default();
    let role = node
        .role
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();

    if let Some(title) = target.title.as_deref().filter(|title| !title.is_empty()) {
        if name == title {
            score += 120;
            identity_matched = true;
            matched_by.push("title_exact");
        } else if contains_case_insensitive(name, title) || contains_case_insensitive(title, name) {
            score += 50;
            identity_matched = true;
            matched_by.push("title_contains");
        }
    }

    if let Some(app_id) = target.app_id.as_deref().filter(|app_id| !app_id.is_empty()) {
        if name == app_id {
            score += 60;
            identity_matched = true;
            matched_by.push("app_id_exact");
        } else if contains_case_insensitive(name, app_id) || contains_case_insensitive(app_id, name)
        {
            score += 25;
            identity_matched = true;
            matched_by.push("app_id_contains");
        }
    }

    if role.contains("frame") || role.contains("window") || role.contains("dialog") {
        score += 10;
        matched_by.push("window_role");
    } else if role.contains("application") {
        score += 3;
        matched_by.push("application_role");
    }

    MatchScore {
        score,
        identity_matched,
        matched_by: matched_by.join("+"),
    }
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    if haystack.is_empty() || needle.is_empty() {
        return false;
    }
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn rect_from_extents((x, y, width, height): (i32, i32, i32, i32)) -> JsonRectI32 {
    JsonRectI32 {
        x,
        y,
        width,
        height,
    }
}

fn optional_string(result: Result<String, impl std::fmt::Display>) -> Option<String> {
    result.ok().filter(|value| !value.is_empty())
}

#[derive(Debug)]
struct MatchedTree {
    score: i32,
    matched_by: String,
    node_count: usize,
    truncated: bool,
    tree: AtspiNode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchScore {
    score: i32,
    identity_matched: bool,
    matched_by: String,
}

impl MatchScore {
    fn is_better_than(&self, other: &Self) -> bool {
        match (self.identity_matched, other.identity_matched) {
            (true, false) => true,
            (false, true) => false,
            _ => self.score > other.score,
        }
    }
}

#[derive(Debug)]
struct TreeBudget {
    max_nodes: usize,
    visited: usize,
    truncated: bool,
}

impl TreeBudget {
    fn new(max_nodes: usize) -> Self {
        Self {
            max_nodes,
            visited: 0,
            truncated: false,
        }
    }

    fn enter(&mut self) -> bool {
        if self.exhausted() {
            self.truncated = true;
            return false;
        }
        self.visited += 1;
        true
    }

    fn exhausted(&self) -> bool {
        self.visited >= self.max_nodes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> WindowAccessibilityTarget {
        WindowAccessibilityTarget {
            id: "window-1".to_string(),
            workspace: 1,
            title: Some("恢复浏览状态 — Mozilla Firefox".to_string()),
            app_id: Some("firefox".to_string()),
            focused: true,
            human_control: false,
            visible: true,
            layout_geometry: JsonRectI32 {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
        }
    }

    fn node(name: &str, role: &str) -> AtspiNode {
        AtspiNode {
            bus_name: Some(":1.1".to_string()),
            path: "/org/a11y/atspi/accessible/root".to_string(),
            depth: 0,
            name: Some(name.to_string()),
            role: Some(role.to_string()),
            role_debug: None,
            description: None,
            child_count: Some(0),
            interfaces: Vec::new(),
            attributes: BTreeMap::new(),
            component: None,
            children: Vec::new(),
            errors: Vec::new(),
        }
    }

    #[test]
    fn role_only_match_is_not_identity_match() {
        let score = score_node(&node("xdg-desktop-portal-gtk", "application"), &target());

        assert_eq!(score.score, 3);
        assert!(!score.identity_matched);
        assert_eq!(score.matched_by, "application_role");
    }

    #[test]
    fn title_match_is_identity_match_with_role_bonus() {
        let score = score_node(&node("恢复浏览状态 — Mozilla Firefox", "frame"), &target());

        assert!(score.identity_matched);
        assert!(score.score > 120);
        assert!(score.matched_by.contains("title_exact"));
        assert!(score.matched_by.contains("window_role"));
    }

    #[test]
    fn app_id_match_is_identity_match() {
        let score = score_node(&node("firefox", "application"), &target());

        assert!(score.identity_matched);
        assert!(score.score > 60);
        assert!(score.matched_by.contains("app_id_exact"));
        assert!(score.matched_by.contains("application_role"));
    }

    #[test]
    fn identity_match_beats_higher_role_only_score() {
        let identity = MatchScore {
            score: 25,
            identity_matched: true,
            matched_by: "app_id_contains".to_string(),
        };
        let role_only = MatchScore {
            score: 40,
            identity_matched: false,
            matched_by: "window_role".to_string(),
        };

        assert!(identity.is_better_than(&role_only));
        assert!(!role_only.is_better_than(&identity));
    }
}
