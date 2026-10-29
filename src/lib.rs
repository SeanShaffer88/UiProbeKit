//! UiProbeKit — pure logic core. Platform access lives in
//! `platform`; the C ABI lives in `ffi`. Keep this module
//! OS-independent and tested.
//!
//! What the core does:
//! - **Spec parsing**: the three JSON specs (`up_inspect`,
//!   `up_find`, `up_action`) are parsed and validated here, on
//!   every platform, before any platform call — a bad spec is
//!   an argument error everywhere, never a platform error.
//! - **Tree model + serialization**: the platform layer builds
//!   a [`TreeNode`] from the live UI Automation tree; turning it
//!   into the documented JSON (with the depth/node caps and the
//!   `truncated` flag) is pure and tested here.
//! - **Find**: matching a [`FindSpec`] against a built tree and
//!   reporting root-relative child-index paths is pure.
//!
//! The walk caps are enforced twice on purpose: the platform
//! walk stops at them (a hostile tree cannot turn an inspect
//! into unbounded work) and the serializer enforces them again
//! on whatever tree it is handed.

pub mod ffi;
pub mod json;
pub mod platform;

pub const VERSION: &str = "0.1.0";
pub const VERSION_MAJOR: u32 = 0;
pub const VERSION_MINOR: u32 = 1;
pub const VERSION_PATCH: u32 = 0;

// Error codes (mirror include/uiprobekit.h).
pub const UP_OK: i32 = 0;
pub const ERR_INVALID_ARG: i32 = 1;
pub const ERR_NOT_SUPPORTED: i32 = 2;
pub const ERR_NOT_FOUND: i32 = 3;
pub const ERR_ACCESS_DENIED: i32 = 4;
pub const ERR_PLATFORM: i32 = 5;
pub const ERR_INTERNAL: i32 = 6;

pub type UpResult<T> = Result<T, (i32, String)>;

fn invalid<T>(msg: impl Into<String>) -> UpResult<T> {
    Err((ERR_INVALID_ARG, msg.into()))
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// A rectangle in screen pixels, origin + size form (the JSON
/// shape every UiProbeKit result uses).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

impl Rect {
    /// From Win32 corner form (left, top, right, bottom).
    pub fn from_corners(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            x: left as i64,
            y: top as i64,
            w: (right - left) as i64,
            h: (bottom - top) as i64,
        }
    }

    pub fn to_value(&self) -> json::Value {
        json::obj(vec![
            ("x", json::Value::I64(self.x)),
            ("y", json::Value::I64(self.y)),
            ("w", json::Value::I64(self.w)),
            ("h", json::Value::I64(self.h)),
        ])
    }
}

// ---------------------------------------------------------------------------
// Windows list model
// ---------------------------------------------------------------------------

/// One top-level window, as `up_list_windows` reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowInfo {
    pub hwnd: i64,
    pub title: String,
    pub class_name: String,
    pub pid: u32,
    pub rect: Rect,
    pub visible: bool,
}

impl WindowInfo {
    pub fn to_value(&self) -> json::Value {
        json::obj(vec![
            ("hwnd", json::Value::I64(self.hwnd)),
            ("title", json::s(&self.title)),
            ("class_name", json::s(&self.class_name)),
            ("pid", json::n(self.pid as u64)),
            ("rect", self.rect.to_value()),
            ("visible", json::b(self.visible)),
        ])
    }
}

// ---------------------------------------------------------------------------
// Control tree model
// ---------------------------------------------------------------------------

/// The UiProbeKit view of one UI Automation element.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeInfo {
    pub name: String,
    /// UIA control type id (e.g. 50000 = Button, 50004 = Edit).
    pub control_type: i32,
    pub automation_id: String,
    pub class_name: String,
    pub rect: Rect,
    pub enabled: bool,
    pub offscreen: bool,
}

impl NodeInfo {
    fn to_value(&self, children: json::Value) -> json::Value {
        json::obj(vec![
            ("name", json::s(&self.name)),
            ("control_type", json::Value::I64(self.control_type as i64)),
            ("automation_id", json::s(&self.automation_id)),
            ("class_name", json::s(&self.class_name)),
            ("rect", self.rect.to_value()),
            ("enabled", json::b(self.enabled)),
            ("offscreen", json::b(self.offscreen)),
            ("children", children),
        ])
    }
}

/// A built control tree. The platform layer produces this from
/// the live UIA tree; everything downstream is pure.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeNode {
    pub info: NodeInfo,
    pub children: Vec<TreeNode>,
}

// ---------------------------------------------------------------------------
// Walk caps
// ---------------------------------------------------------------------------

pub const DEFAULT_MAX_DEPTH: usize = 8;
pub const DEFAULT_MAX_NODES: usize = 2000;
/// Hard bounds a spec may ask for (validated at parse time).
pub const MAX_DEPTH_LIMIT: usize = 256;
pub const MAX_NODES_LIMIT: usize = 1_000_000;
/// `up_find` has no caps in its spec; it searches within these
/// internal bounds so a find cannot walk forever either.
pub const FIND_MAX_DEPTH: usize = 64;
pub const FIND_MAX_NODES: usize = 100_000;
/// Longest action path the parser accepts (child indexes from
/// the root element).
pub const MAX_PATH_LEN: usize = 512;
/// Largest single child index the parser accepts.
pub const MAX_PATH_INDEX: i64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalkLimits {
    pub max_depth: usize,
    pub max_nodes: usize,
}

impl WalkLimits {
    pub fn inspect_defaults() -> WalkLimits {
        WalkLimits {
            max_depth: DEFAULT_MAX_DEPTH,
            max_nodes: DEFAULT_MAX_NODES,
        }
    }
    pub fn find_defaults() -> WalkLimits {
        WalkLimits {
            max_depth: FIND_MAX_DEPTH,
            max_nodes: FIND_MAX_NODES,
        }
    }
}

/// Cap bookkeeping shared by the platform walk. `admit` is
/// called once per node, in pre-order; the first refusal sets
/// `truncated` and every later refusal keeps it set.
#[derive(Debug)]
pub struct WalkState {
    pub limits: WalkLimits,
    pub count: usize,
    pub truncated: bool,
}

impl WalkState {
    pub fn new(limits: WalkLimits) -> WalkState {
        WalkState {
            limits,
            count: 0,
            truncated: false,
        }
    }

    /// Admit one node at `depth` (the root is depth 0). Returns
    /// false — and records truncation — when a cap refuses it.
    pub fn admit(&mut self, depth: usize) -> bool {
        if depth > self.limits.max_depth || self.count >= self.limits.max_nodes {
            self.truncated = true;
            return false;
        }
        self.count += 1;
        true
    }

    /// Record that content existed beyond a cap (e.g. a node at
    /// the depth cap turned out to have children).
    pub fn cut(&mut self) {
        self.truncated = true;
    }
}

// ---------------------------------------------------------------------------
// Tree serialization (caps enforced here as well)
// ---------------------------------------------------------------------------

/// Serialize a built tree to the `up_inspect` result shape: the
/// root node object plus a top-level `"truncated"` flag that is
/// true when either cap cut anything. Returns (value, truncated).
pub fn serialize_tree(root: &TreeNode, limits: WalkLimits) -> (json::Value, bool) {
    let mut state = WalkState::new(limits);
    let value = ser_node(root, 0, &mut state).unwrap_or(json::Value::Null);
    (value, state.truncated)
}

fn ser_node(node: &TreeNode, depth: usize, state: &mut WalkState) -> Option<json::Value> {
    if !state.admit(depth) {
        return None;
    }
    let mut children = Vec::new();
    if depth >= state.limits.max_depth {
        if !node.children.is_empty() {
            state.cut();
        }
    } else {
        for child in &node.children {
            match ser_node(child, depth + 1, state) {
                Some(v) => children.push(v),
                None => break, // node cap hit; admit() set the flag
            }
        }
    }
    Some(node.info.to_value(json::Value::Array(children)))
}

// ---------------------------------------------------------------------------
// Spec parsing
// ---------------------------------------------------------------------------

fn parse_object(text: &str, what: &str) -> UpResult<json::Value> {
    let v = json::parse(text).map_err(|e| (ERR_INVALID_ARG, format!("{what}: {e}")))?;
    if v.as_object().is_none() {
        return invalid(format!("{what} must be a JSON object"));
    }
    Ok(v)
}

fn reject_unknown_keys(v: &json::Value, allowed: &[&str], what: &str) -> UpResult<()> {
    for key in v.keys() {
        if !allowed.contains(&key) {
            return invalid(format!("{what}: unknown key \"{key}\""));
        }
    }
    Ok(())
}

fn req_hwnd(v: &json::Value, what: &str) -> UpResult<i64> {
    match v.get("hwnd") {
        None => invalid(format!("{what}: \"hwnd\" is required")),
        Some(h) => match h.as_i64() {
            Some(n) if n > 0 => Ok(n),
            Some(_) => invalid(format!("{what}: \"hwnd\" must be a positive integer")),
            None => invalid(format!(
                "{what}: \"hwnd\" must be an integer, got {}",
                h.type_name()
            )),
        },
    }
}

fn opt_string(v: &json::Value, key: &str, what: &str) -> UpResult<Option<String>> {
    match v.get(key) {
        None => Ok(None),
        Some(s) => match s.as_str() {
            Some(s) => Ok(Some(s.to_string())),
            None => invalid(format!(
                "{what}: \"{key}\" must be a string, got {}",
                s.type_name()
            )),
        },
    }
}

fn opt_usize(
    v: &json::Value,
    key: &str,
    default: usize,
    max: usize,
    what: &str,
) -> UpResult<usize> {
    match v.get(key) {
        None => Ok(default),
        Some(n) => match n.as_i64() {
            Some(n) if n >= 0 && (n as usize) <= max => Ok(n as usize),
            Some(_) => invalid(format!("{what}: \"{key}\" must be between 0 and {max}")),
            None => invalid(format!(
                "{what}: \"{key}\" must be an integer, got {}",
                n.type_name()
            )),
        },
    }
}

/// Parse a root-relative path: an array of child indexes, each
/// a non-negative integer. The empty path addresses the root
/// element itself.
pub fn parse_path(v: &json::Value, what: &str) -> UpResult<Vec<usize>> {
    let items = match v.as_array() {
        Some(items) => items,
        None => {
            return invalid(format!(
                "{what}: \"path\" must be an array of child indexes, got {}",
                v.type_name()
            ))
        }
    };
    if items.len() > MAX_PATH_LEN {
        return invalid(format!(
            "{what}: \"path\" has {} steps; the limit is {MAX_PATH_LEN}",
            items.len()
        ));
    }
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        match item.as_i64() {
            Some(n) if (0..=MAX_PATH_INDEX).contains(&n) => out.push(n as usize),
            Some(_) => {
                return invalid(format!(
                    "{what}: \"path\" step {i} must be between 0 and {MAX_PATH_INDEX}"
                ))
            }
            None => {
                return invalid(format!(
                    "{what}: \"path\" step {i} must be an integer, got {}",
                    item.type_name()
                ))
            }
        }
    }
    Ok(out)
}

// -- up_inspect --------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct InspectSpec {
    pub hwnd: i64,
    pub limits: WalkLimits,
}

pub fn parse_inspect_spec(text: &str) -> UpResult<InspectSpec> {
    let what = "inspect spec";
    let v = parse_object(text, what)?;
    reject_unknown_keys(&v, &["hwnd", "max_depth", "max_nodes"], what)?;
    let hwnd = req_hwnd(&v, what)?;
    let max_depth = opt_usize(&v, "max_depth", DEFAULT_MAX_DEPTH, MAX_DEPTH_LIMIT, what)?;
    let max_nodes = opt_usize(&v, "max_nodes", DEFAULT_MAX_NODES, MAX_NODES_LIMIT, what)?;
    if max_nodes == 0 {
        return invalid(format!("{what}: \"max_nodes\" must be at least 1"));
    }
    Ok(InspectSpec {
        hwnd,
        limits: WalkLimits {
            max_depth,
            max_nodes,
        },
    })
}

// -- up_find -----------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct FindSpec {
    pub hwnd: i64,
    pub name: Option<String>,
    pub automation_id: Option<String>,
    pub control_type: Option<i32>,
}

pub fn parse_find_spec(text: &str) -> UpResult<FindSpec> {
    let what = "find spec";
    let v = parse_object(text, what)?;
    reject_unknown_keys(&v, &["hwnd", "name", "automation_id", "control_type"], what)?;
    let hwnd = req_hwnd(&v, what)?;
    let name = opt_string(&v, "name", what)?;
    let automation_id = opt_string(&v, "automation_id", what)?;
    let control_type = match v.get("control_type") {
        None => None,
        Some(n) => match n.as_i64() {
            Some(n) if n >= 0 && n <= i32::MAX as i64 => Some(n as i32),
            Some(_) => {
                return invalid(format!(
                    "{what}: \"control_type\" must be a UIA control type id (0..={})",
                    i32::MAX
                ))
            }
            None => {
                return invalid(format!(
                    "{what}: \"control_type\" must be an integer, got {}",
                    n.type_name()
                ))
            }
        },
    };
    if name.is_none() && automation_id.is_none() && control_type.is_none() {
        return invalid(format!(
            "{what}: at least one of \"name\", \"automation_id\", \"control_type\" is required — an unfiltered find would match every element"
        ));
    }
    Ok(FindSpec {
        hwnd,
        name,
        automation_id,
        control_type,
    })
}

/// Does one element satisfy every criterion the spec carries?
/// String criteria compare exactly (case-sensitive); the
/// control type compares by UIA id.
pub fn node_matches(info: &NodeInfo, spec: &FindSpec) -> bool {
    if let Some(name) = &spec.name {
        if info.name != *name {
            return false;
        }
    }
    if let Some(id) = &spec.automation_id {
        if info.automation_id != *id {
            return false;
        }
    }
    if let Some(ct) = spec.control_type {
        if info.control_type != ct {
            return false;
        }
    }
    true
}

/// All root-relative child-index paths in `root` whose node
/// matches `spec`, in pre-order. The root itself is path `[]`.
pub fn find_paths(root: &TreeNode, spec: &FindSpec) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    find_into(root, spec, &mut path, &mut out);
    out
}

fn find_into(node: &TreeNode, spec: &FindSpec, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    if node_matches(&node.info, spec) {
        out.push(path.clone());
    }
    for (i, child) in node.children.iter().enumerate() {
        path.push(i);
        find_into(child, spec, path, out);
        path.pop();
    }
}

// -- up_action ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Invoke,
    Toggle,
    SetValue,
    Focus,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActionSpec {
    pub hwnd: i64,
    pub path: Vec<usize>,
    pub action: ActionKind,
    /// Required for `set_value`; ignored by the other actions.
    pub value: Option<String>,
}

pub fn parse_action_spec(text: &str) -> UpResult<ActionSpec> {
    let what = "action spec";
    let v = parse_object(text, what)?;
    reject_unknown_keys(&v, &["hwnd", "path", "action", "value"], what)?;
    let hwnd = req_hwnd(&v, what)?;
    let path = match v.get("path") {
        None => return invalid(format!("{what}: \"path\" is required")),
        Some(p) => parse_path(p, what)?,
    };
    let action = match v.get("action") {
        None => return invalid(format!("{what}: \"action\" is required")),
        Some(a) => match a.as_str() {
            Some("invoke") => ActionKind::Invoke,
            Some("toggle") => ActionKind::Toggle,
            Some("set_value") => ActionKind::SetValue,
            Some("focus") => ActionKind::Focus,
            Some(other) => {
                return invalid(format!(
                    "{what}: \"action\" must be \"invoke\", \"toggle\", \"set_value\" or \"focus\", got \"{other}\""
                ))
            }
            None => {
                return invalid(format!(
                    "{what}: \"action\" must be a string, got {}",
                    a.type_name()
                ))
            }
        },
    };
    let value = opt_string(&v, "value", what)?;
    if action == ActionKind::SetValue && value.is_none() {
        return invalid(format!(
            "{what}: \"value\" is required for the \"set_value\" action"
        ));
    }
    Ok(ActionSpec {
        hwnd,
        path,
        action,
        value,
    })
}

// ---------------------------------------------------------------------------
// Entry points (spec in, JSON out — the FFI wraps these)
// ---------------------------------------------------------------------------

/// `up_list_windows`: every top-level window as a JSON array.
pub fn list_windows() -> UpResult<String> {
    let windows = platform::list_windows()?;
    let items: Vec<json::Value> = windows.iter().map(WindowInfo::to_value).collect();
    Ok(json::Value::Array(items).to_json())
}

/// `up_inspect`: the control tree under a window, as JSON.
pub fn inspect(spec_json: &str) -> UpResult<String> {
    let spec = parse_inspect_spec(spec_json)?;
    let (tree, walk_truncated) = platform::inspect_tree(spec.hwnd, spec.limits)?;
    let (mut value, ser_truncated) = serialize_tree(&tree, spec.limits);
    if let json::Value::Object(pairs) = &mut value {
        pairs.push((
            "truncated".to_string(),
            json::b(walk_truncated || ser_truncated),
        ));
    }
    Ok(value.to_json())
}

/// `up_find`: root-relative paths of every matching element.
pub fn find(spec_json: &str) -> UpResult<String> {
    let spec = parse_find_spec(spec_json)?;
    let (tree, _truncated) = platform::inspect_tree(spec.hwnd, WalkLimits::find_defaults())?;
    let matches: Vec<json::Value> = find_paths(&tree, &spec)
        .iter()
        .map(|p| {
            json::obj(vec![(
                "path",
                json::Value::Array(p.iter().map(|&i| json::Value::I64(i as i64)).collect()),
            )])
        })
        .collect();
    Ok(json::Value::Array(matches).to_json())
}

/// `up_action`: perform one action on the element a path
/// addresses. Returns the FFI status code via UpResult.
pub fn action(spec_json: &str) -> UpResult<()> {
    let spec = parse_action_spec(spec_json)?;
    platform::perform_action(&spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, ct: i32, children: Vec<TreeNode>) -> TreeNode {
        TreeNode {
            info: NodeInfo {
                name: name.to_string(),
                control_type: ct,
                automation_id: String::new(),
                class_name: String::new(),
                rect: Rect::default(),
                enabled: true,
                offscreen: false,
            },
            children,
        }
    }

    fn sample_tree() -> TreeNode {
        // Window
        // ├─ [0] TitleBar (50037)
        // └─ [1] Pane
        //     ├─ [0] Save (Button 50000, id "saveBtn")
        //     └─ [1] Name (Edit 50004)
        let mut save = node("Save", 50000, vec![]);
        save.info.automation_id = "saveBtn".to_string();
        node(
            "Main",
            50032,
            vec![
                node("TitleBar", 50037, vec![]),
                node("Pane", 50033, vec![save, node("Name", 50004, vec![])]),
            ],
        )
    }

    // -- spec parsing ---------------------------------------------------------

    #[test]
    fn inspect_spec_defaults_and_overrides() {
        let s = parse_inspect_spec(r#"{"hwnd": 1234}"#).unwrap();
        assert_eq!(s.hwnd, 1234);
        assert_eq!(s.limits.max_depth, DEFAULT_MAX_DEPTH);
        assert_eq!(s.limits.max_nodes, DEFAULT_MAX_NODES);
        let s = parse_inspect_spec(r#"{"hwnd": 7, "max_depth": 0, "max_nodes": 1}"#).unwrap();
        assert_eq!(s.limits.max_depth, 0);
        assert_eq!(s.limits.max_nodes, 1);
    }

    #[test]
    fn inspect_spec_rejects_bad_input() {
        for bad in [
            "not json",
            "[]",
            "{}",
            r#"{"hwnd": 0}"#,
            r#"{"hwnd": -3}"#,
            r#"{"hwnd": "12"}"#,
            r#"{"hwnd": 1, "max_depth": 257}"#,
            r#"{"hwnd": 1, "max_nodes": 0}"#,
            r#"{"hwnd": 1, "max_nodes": 1000001}"#,
            r#"{"hwnd": 1, "bogus": 2}"#,
            r#"{"hwnd": 1, "hwnd": 2}"#, // duplicate key: parser-level error
        ] {
            let e = parse_inspect_spec(bad).unwrap_err();
            assert_eq!(e.0, ERR_INVALID_ARG, "input {bad}");
        }
    }

    #[test]
    fn find_spec_validation() {
        let s = parse_find_spec(r#"{"hwnd": 9, "name": "Save"}"#).unwrap();
        assert_eq!(s.name.as_deref(), Some("Save"));
        let s = parse_find_spec(r#"{"hwnd": 9, "control_type": 50000}"#).unwrap();
        assert_eq!(s.control_type, Some(50000));
        // No criterion at all: refused, it would match everything.
        assert_eq!(
            parse_find_spec(r#"{"hwnd": 9}"#).unwrap_err().0,
            ERR_INVALID_ARG
        );
        assert_eq!(
            parse_find_spec(r#"{"hwnd": 9, "name": 3}"#).unwrap_err().0,
            ERR_INVALID_ARG
        );
        assert_eq!(
            parse_find_spec(r#"{"hwnd": 9, "control_type": -1}"#)
                .unwrap_err()
                .0,
            ERR_INVALID_ARG
        );
    }

    #[test]
    fn action_spec_validation() {
        let s = parse_action_spec(r#"{"hwnd": 9, "path": [], "action": "focus"}"#).unwrap();
        assert_eq!(s.action, ActionKind::Focus);
        assert!(s.path.is_empty());
        let s = parse_action_spec(
            r#"{"hwnd": 9, "path": [1, 0], "action": "set_value", "value": "abc"}"#,
        )
        .unwrap();
        assert_eq!(s.path, vec![1, 0]);
        assert_eq!(s.value.as_deref(), Some("abc"));
        // set_value without a value is an argument error.
        assert_eq!(
            parse_action_spec(r#"{"hwnd": 9, "path": [], "action": "set_value"}"#)
                .unwrap_err()
                .0,
            ERR_INVALID_ARG
        );
        // Unknown action.
        assert_eq!(
            parse_action_spec(r#"{"hwnd": 9, "path": [], "action": "click"}"#)
                .unwrap_err()
                .0,
            ERR_INVALID_ARG
        );
        // Missing path.
        assert_eq!(
            parse_action_spec(r#"{"hwnd": 9, "action": "invoke"}"#)
                .unwrap_err()
                .0,
            ERR_INVALID_ARG
        );
    }

    #[test]
    fn path_parsing_and_validation() {
        let v = json::parse("[0, 12, 3]").unwrap();
        assert_eq!(parse_path(&v, "t").unwrap(), vec![0, 12, 3]);
        let v = json::parse("[]").unwrap();
        assert_eq!(parse_path(&v, "t").unwrap(), Vec::<usize>::new());
        for bad in ["[-1]", "[1.5]", "[\"0\"]", "[null]", "3", "\"x\""] {
            let v = json::parse(bad).unwrap();
            assert_eq!(parse_path(&v, "t").unwrap_err().0, ERR_INVALID_ARG, "{bad}");
        }
        let too_long = json::Value::Array(vec![json::Value::I64(0); MAX_PATH_LEN + 1]);
        assert_eq!(parse_path(&too_long, "t").unwrap_err().0, ERR_INVALID_ARG);
    }

    // -- tree serialization + caps ---------------------------------------------

    #[test]
    fn serialize_tree_shape() {
        let (v, truncated) = serialize_tree(&sample_tree(), WalkLimits::inspect_defaults());
        assert!(!truncated);
        assert_eq!(v.get("name").unwrap().as_str(), Some("Main"));
        assert_eq!(v.get("control_type").unwrap().as_i64(), Some(50032));
        let children = v.get("children").unwrap().as_array().unwrap();
        assert_eq!(children.len(), 2);
        let save = &children[1].get("children").unwrap().as_array().unwrap()[0];
        assert_eq!(save.get("automation_id").unwrap().as_str(), Some("saveBtn"));
        assert_eq!(save.get("enabled").unwrap().as_bool(), Some(true));
        let rect = save.get("rect").unwrap();
        assert_eq!(rect.get("w").unwrap().as_i64(), Some(0));
    }

    #[test]
    fn serialize_tree_depth_cap_marks_truncated() {
        let limits = WalkLimits {
            max_depth: 1,
            max_nodes: 2000,
        };
        let (v, truncated) = serialize_tree(&sample_tree(), limits);
        assert!(truncated, "nodes below depth 1 were cut");
        let children = v.get("children").unwrap().as_array().unwrap();
        assert_eq!(children.len(), 2);
        // The Pane at depth 1 is present but its children are cut.
        let pane_children = children[1].get("children").unwrap().as_array().unwrap();
        assert!(pane_children.is_empty());
    }

    #[test]
    fn serialize_tree_node_cap_marks_truncated() {
        let limits = WalkLimits {
            max_depth: 8,
            max_nodes: 3,
        };
        let (v, truncated) = serialize_tree(&sample_tree(), limits);
        assert!(truncated);
        // Pre-order admission: root, TitleBar, Pane — then the cap.
        let children = v.get("children").unwrap().as_array().unwrap();
        assert_eq!(children.len(), 2);
        let pane_children = children[1].get("children").unwrap().as_array().unwrap();
        assert!(pane_children.is_empty());
        // Exactly at the cap with nothing beyond: NOT truncated.
        let exact = WalkLimits {
            max_depth: 8,
            max_nodes: 5,
        };
        let (_, truncated) = serialize_tree(&sample_tree(), exact);
        assert!(!truncated);
    }

    #[test]
    fn walk_state_admission() {
        let mut st = WalkState::new(WalkLimits {
            max_depth: 1,
            max_nodes: 2,
        });
        assert!(st.admit(0));
        assert!(st.admit(1));
        assert!(!st.admit(1)); // node cap
        assert!(st.truncated);
        let mut st = WalkState::new(WalkLimits {
            max_depth: 0,
            max_nodes: 10,
        });
        assert!(st.admit(0));
        assert!(!st.admit(1)); // depth cap
        assert!(st.truncated);
    }

    // -- find -------------------------------------------------------------------

    #[test]
    fn find_paths_in_tree() {
        let tree = sample_tree();
        let by_name = parse_find_spec(r#"{"hwnd": 1, "name": "Save"}"#).unwrap();
        assert_eq!(find_paths(&tree, &by_name), vec![vec![1, 0]]);
        let by_id = parse_find_spec(r#"{"hwnd": 1, "automation_id": "saveBtn"}"#).unwrap();
        assert_eq!(find_paths(&tree, &by_id), vec![vec![1, 0]]);
        let by_type = parse_find_spec(r#"{"hwnd": 1, "control_type": 50004}"#).unwrap();
        assert_eq!(find_paths(&tree, &by_type), vec![vec![1, 1]]);
        // Combined criteria are ANDed.
        let both =
            parse_find_spec(r#"{"hwnd": 1, "name": "Save", "control_type": 50004}"#).unwrap();
        assert!(find_paths(&tree, &both).is_empty());
        // The root itself can match: its path is [].
        let root = parse_find_spec(r#"{"hwnd": 1, "name": "Main"}"#).unwrap();
        assert_eq!(find_paths(&tree, &root), vec![Vec::<usize>::new()]);
        // No match: empty result, not an error.
        let none = parse_find_spec(r#"{"hwnd": 1, "name": "Nope"}"#).unwrap();
        assert!(find_paths(&tree, &none).is_empty());
    }

    // -- window list shape -------------------------------------------------------

    #[test]
    fn window_info_shape() {
        let w = WindowInfo {
            hwnd: 42,
            title: "Doc — Editor".to_string(),
            class_name: "CabinetWClass".to_string(),
            pid: 1234,
            rect: Rect::from_corners(10, 20, 110, 70),
            visible: true,
        };
        let v = w.to_value();
        assert_eq!(v.get("hwnd").unwrap().as_i64(), Some(42));
        assert_eq!(v.get("pid").unwrap().as_i64(), Some(1234));
        let r = v.get("rect").unwrap();
        assert_eq!(r.get("x").unwrap().as_i64(), Some(10));
        assert_eq!(r.get("w").unwrap().as_i64(), Some(100));
        assert_eq!(r.get("h").unwrap().as_i64(), Some(50));
        // The serialized text round-trips through the parser.
        let back = json::parse(&v.to_json()).unwrap();
        assert_eq!(back.get("title").unwrap().as_str(), Some("Doc — Editor"));
    }

    // -- platform contract off Windows --------------------------------------------

    #[test]
    fn platform_contract_off_windows() {
        // Argument errors are raised by the pure parsers on every
        // platform, before any platform call.
        assert_eq!(inspect("nope").unwrap_err().0, ERR_INVALID_ARG);
        assert_eq!(find("{}").unwrap_err().0, ERR_INVALID_ARG);
        assert_eq!(action("{}").unwrap_err().0, ERR_INVALID_ARG);
        #[cfg(not(windows))]
        {
            // Valid specs reach the platform layer, which is a
            // clean NOT_SUPPORTED — never a fake result.
            assert_eq!(list_windows().unwrap_err().0, ERR_NOT_SUPPORTED);
            assert_eq!(inspect(r#"{"hwnd": 5}"#).unwrap_err().0, ERR_NOT_SUPPORTED);
            assert_eq!(
                find(r#"{"hwnd": 5, "name": "x"}"#).unwrap_err().0,
                ERR_NOT_SUPPORTED
            );
            assert_eq!(
                action(r#"{"hwnd": 5, "path": [], "action": "focus"}"#)
                    .unwrap_err()
                    .0,
                ERR_NOT_SUPPORTED
            );
        }
    }
}
