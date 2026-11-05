//! Click params -> closed JSON Schema + ArgMappings (architecture §11, ADR 0009).
//!
//! Type mapping: String -> string, Int/IntRange -> integer (+bounds), Float/FloatRange -> number (+bounds),
//! Bool/flag -> boolean, Choice -> enum, Path/File -> `format: path` (+ a path scope), `multiple` -> array
//! (repeated option), `nargs=-1` -> positional array. The CLI-Anything globals are never exposed: `--json`
//! goes into `global_args`, `--project` becomes the engine-supplied state handle, `--dry-run` becomes
//! `DryRun{NO_PERSIST}`.

use crate::common::{json_pointer, object_schema};
use aa_core::{AaError, AaResult, pb};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Default)]
pub struct CliAnythingGlobals {
    pub json_flag: Option<String>,
    pub project_flag: Option<String>,
    pub dry_run_flag: Option<String>,
}

/// Click param list (to_info_dict) -> closed JSON Schema + ArgMappings; global --json/--project/--dry-run
/// handled per §11.
pub fn click_params_to_schema(
    params: &serde_json::Value,
    globals: &CliAnythingGlobals,
) -> aa_core::AaResult<(serde_json::Value, Vec<aa_core::pb::ArgMapping>)> {
    let a = analyze(params, globals)?;
    Ok((a.schema, a.args))
}

/// Whether a path parameter is read or written (decides the scope kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathUse {
    Read,
    Write,
}

pub(crate) struct Analysis {
    pub schema: Value,
    pub args: Vec<pb::ArgMapping>,
    /// (JSON pointer, use) for every Path/File parameter.
    pub paths: Vec<(String, PathUse)>,
}

/// Detects the CLI-Anything globals on a root group's params (`--json`, `--project`, `--dry-run`).
pub(crate) fn detect_globals(root_params: &Value) -> CliAnythingGlobals {
    let mut g = CliAnythingGlobals::default();
    for p in root_params.as_array().into_iter().flatten() {
        if p.get("param_type_name").and_then(Value::as_str) != Some("option") {
            continue;
        }
        let is_flag = p.get("is_flag").and_then(Value::as_bool).unwrap_or(false);
        for opt in strings(p.get("opts")) {
            match (opt.as_str(), is_flag) {
                ("--json", true) => g.json_flag = Some(opt.clone()),
                ("--project", false) => g.project_flag = Some(opt.clone()),
                ("--dry-run", true) => g.dry_run_flag = Some(opt.clone()),
                _ => {}
            }
        }
    }
    g
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

fn is_global(opts: &[String], globals: &CliAnythingGlobals) -> bool {
    [&globals.json_flag, &globals.project_flag, &globals.dry_run_flag]
        .into_iter()
        .flatten()
        .any(|g| opts.iter().any(|o| o == g))
}

pub(crate) fn analyze(params: &Value, globals: &CliAnythingGlobals) -> AaResult<Analysis> {
    let list = params
        .as_array()
        .ok_or_else(|| AaError::invalid("click params must be a JSON array (to_info_dict()[\"params\"])"))?;
    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut args = Vec::new();
    let mut paths = Vec::new();
    let mut position = 0u32;
    for p in list {
        let kind = p.get("param_type_name").and_then(Value::as_str).unwrap_or("");
        let opts = strings(p.get("opts"));
        if p.get("hidden").and_then(Value::as_bool).unwrap_or(false)
            || opts.iter().any(|o| o == "--help")
            || is_global(&opts, globals)
        {
            continue;
        }
        let Some(name) = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()) else {
            continue;
        };
        if properties.contains_key(name) {
            continue;
        }
        let pointer = json_pointer(name);
        let nargs = p.get("nargs").and_then(Value::as_i64).unwrap_or(1);
        let ty = p.get("type").cloned().unwrap_or(Value::Null);
        let (item, path_use) = type_schema(&ty);
        let is_required = p.get("required").and_then(Value::as_bool).unwrap_or(false);
        let (schema, mapping) = match kind {
            "option" => {
                let is_flag = p.get("is_flag").and_then(Value::as_bool).unwrap_or(false);
                let count = p.get("count").and_then(Value::as_bool).unwrap_or(false);
                let multiple = p.get("multiple").and_then(Value::as_bool).unwrap_or(false);
                let flag = opts
                    .iter()
                    .find(|o| o.starts_with("--"))
                    .or_else(|| opts.first())
                    .cloned()
                    .unwrap_or_default();
                if flag.is_empty() {
                    continue;
                }
                let short_only = !flag.starts_with("--");
                let secondary = strings(p.get("secondary_opts"));
                if is_flag || count {
                    let style = if is_flag && !secondary.is_empty() {
                        pb::ArgStyle::NegatableFlag
                    } else {
                        pb::ArgStyle::Flag
                    };
                    (json!({"type": "boolean"}), mapping(&pointer, style, &flag, 0, false))
                } else if nargs != 1 {
                    // Tuple-valued options (`nargs>1`) have no argv style that keeps values unambiguous.
                    tracing::debug!(param = name, nargs, "click-info: skipping tuple-valued option");
                    continue;
                } else if multiple {
                    (
                        json!({"type": "array", "items": item}),
                        mapping(&pointer, pb::ArgStyle::RepeatedOption, &flag, 0, short_only),
                    )
                } else {
                    (item, mapping(&pointer, pb::ArgStyle::Option, &flag, 0, short_only))
                }
            }
            "argument" => {
                let schema = match nargs {
                    1 => item,
                    n if n < 0 => {
                        let mut s = json!({"type": "array", "items": item});
                        if is_required {
                            s["minItems"] = json!(1);
                        }
                        s
                    }
                    n => json!({"type": "array", "items": item, "minItems": n, "maxItems": n}),
                };
                let m = mapping(&pointer, pb::ArgStyle::Positional, "", position, false);
                position += 1;
                (schema, m)
            }
            _ => continue,
        };
        if let Some(u) = path_use {
            paths.push((pointer.clone(), u));
        }
        properties.insert(name.to_owned(), schema);
        if is_required {
            required.push(name.to_owned());
        }
        args.push(mapping);
    }
    Ok(Analysis {
        schema: object_schema(properties, required),
        args,
        paths,
    })
}

fn mapping(pointer: &str, style: pb::ArgStyle, flag: &str, position: u32, no_equals_form: bool) -> pb::ArgMapping {
    pb::ArgMapping {
        pointer: pointer.to_owned(),
        style: style as i32,
        flag: flag.to_owned(),
        position,
        no_equals_form,
    }
}

fn bounds(schema: &mut Value, ty: &Value) {
    let open = |k: &str| ty.get(k).and_then(Value::as_bool).unwrap_or(false);
    if let Some(min) = ty.get("min").filter(|v| v.is_number()) {
        let key = if open("min_open") {
            "exclusiveMinimum"
        } else {
            "minimum"
        };
        schema[key] = min.clone();
    }
    if let Some(max) = ty.get("max").filter(|v| v.is_number()) {
        let key = if open("max_open") {
            "exclusiveMaximum"
        } else {
            "maximum"
        };
        schema[key] = max.clone();
    }
}

/// JSON Schema for one Click `ParamType.to_info_dict()`, plus whether it designates a path.
fn type_schema(ty: &Value) -> (Value, Option<PathUse>) {
    let param_type = ty.get("param_type").and_then(Value::as_str).unwrap_or("String");
    match param_type {
        "Int" => (json!({"type": "integer"}), None),
        "IntRange" => {
            let mut s = json!({"type": "integer"});
            bounds(&mut s, ty);
            (s, None)
        }
        "Float" => (json!({"type": "number"}), None),
        "FloatRange" => {
            let mut s = json!({"type": "number"});
            bounds(&mut s, ty);
            (s, None)
        }
        "Bool" => (json!({"type": "boolean"}), None),
        "Choice" => {
            let choices: Vec<Value> = ty
                .get("choices")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|c| match c {
                    Value::String(s) => Value::String(s.clone()),
                    other => Value::String(other.to_string()),
                })
                .collect();
            (json!({"type": "string", "enum": choices}), None)
        }
        "Path" => {
            let writable = ty.get("writable").and_then(Value::as_bool).unwrap_or(false);
            let u = if writable { PathUse::Write } else { PathUse::Read };
            (json!({"type": "string", "format": "path"}), Some(u))
        }
        "File" => {
            let mode = ty.get("mode").and_then(Value::as_str).unwrap_or("r");
            let u = if mode.contains(['w', 'a', 'x', '+']) {
                PathUse::Write
            } else {
                PathUse::Read
            };
            (json!({"type": "string", "format": "path"}), Some(u))
        }
        "UUID" => (json!({"type": "string", "format": "uuid"}), None),
        _ => (json!({"type": "string"}), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(name: &str, opts: &[&str], ty: Value) -> Value {
        json!({"param_type_name": "option", "name": name, "opts": opts, "secondary_opts": [], "type": ty,
               "required": false, "nargs": 1, "multiple": false, "is_flag": false, "count": false, "hidden": false})
    }

    fn cli_anything_globals() -> CliAnythingGlobals {
        CliAnythingGlobals {
            json_flag: Some("--json".into()),
            project_flag: Some("--project".into()),
            dry_run_flag: Some("--dry-run".into()),
        }
    }

    fn style(args: &[pb::ArgMapping], pointer: &str) -> pb::ArgStyle {
        let m = args
            .iter()
            .find(|a| a.pointer == pointer)
            .unwrap_or_else(|| panic!("no mapping {pointer}"));
        pb::ArgStyle::try_from(m.style).unwrap()
    }

    #[test]
    fn type_mapping_table() {
        let mut multi = option("tag", &["--tag", "-t"], json!({"param_type": "String"}));
        multi["multiple"] = json!(true);
        let mut flag = option("force", &["--force", "-f"], json!({"param_type": "Bool"}));
        flag["is_flag"] = json!(true);
        let mut negatable = option("color", &["--color"], json!({"param_type": "Bool"}));
        negatable["is_flag"] = json!(true);
        negatable["secondary_opts"] = json!(["--no-color"]);
        let mut req = option("text", &["--text"], json!({"param_type": "String"}));
        req["required"] = json!(true);
        let params = json!([
            option("fmt", &["--format"], json!({"param_type": "Choice", "choices": ["txt", "json"]})),
            option("limit", &["--limit"], json!({"param_type": "IntRange", "min": 1, "max": 1000, "min_open": false, "max_open": true})),
            option("ratio", &["--ratio"], json!({"param_type": "Float"})),
            multi,
            flag,
            negatable,
            req,
            option("output_path", &["--output"], json!({"param_type": "Path", "writable": true})),
            option("n", &["-n"], json!({"param_type": "Int"})),
            {"param_type_name": "argument", "name": "words", "opts": ["words"], "type": {"param_type": "String"},
             "required": false, "nargs": -1, "multiple": false},
            {"param_type_name": "argument", "name": "index", "opts": ["index"], "type": {"param_type": "Int"},
             "required": true, "nargs": 1, "multiple": false},
        ]);
        let (schema, args) = click_params_to_schema(&params, &CliAnythingGlobals::default()).unwrap();
        let p = &schema["properties"];
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(schema["type"], json!("object"));
        assert_eq!(p["fmt"], json!({"type": "string", "enum": ["txt", "json"]}));
        assert_eq!(
            p["limit"],
            json!({"type": "integer", "minimum": 1, "exclusiveMaximum": 1000})
        );
        assert_eq!(p["ratio"], json!({"type": "number"}));
        assert_eq!(p["tag"], json!({"type": "array", "items": {"type": "string"}}));
        assert_eq!(p["force"], json!({"type": "boolean"}));
        assert_eq!(p["color"], json!({"type": "boolean"}));
        assert_eq!(p["output_path"], json!({"type": "string", "format": "path"}));
        assert_eq!(p["words"], json!({"type": "array", "items": {"type": "string"}}));
        assert_eq!(p["index"], json!({"type": "integer"}));
        assert_eq!(schema["required"], json!(["text", "index"]));
        assert_eq!(style(&args, "/fmt"), pb::ArgStyle::Option);
        assert_eq!(style(&args, "/tag"), pb::ArgStyle::RepeatedOption);
        assert_eq!(style(&args, "/force"), pb::ArgStyle::Flag);
        assert_eq!(style(&args, "/color"), pb::ArgStyle::NegatableFlag);
        assert_eq!(style(&args, "/words"), pb::ArgStyle::Positional);
        let tag = args.iter().find(|a| a.pointer == "/tag").unwrap();
        assert_eq!(tag.flag, "--tag", "long form preferred");
        let n = args.iter().find(|a| a.pointer == "/n").unwrap();
        assert!(n.no_equals_form, "short-only options cannot use -n=value");
        let positions: Vec<u32> = args
            .iter()
            .filter(|a| a.style == pb::ArgStyle::Positional as i32)
            .map(|a| a.position)
            .collect();
        assert_eq!(positions, vec![0, 1]);
        let a = analyze(&params, &CliAnythingGlobals::default()).unwrap();
        assert_eq!(a.paths, vec![("/output_path".to_owned(), PathUse::Write)]);
    }

    #[test]
    fn globals_are_never_exposed() {
        let mut json_flag = option("as_json", &["--json"], json!({"param_type": "Bool"}));
        json_flag["is_flag"] = json!(true);
        let mut dry = option("dry_run", &["--dry-run"], json!({"param_type": "Bool"}));
        dry["is_flag"] = json!(true);
        let root = json!([
            json_flag,
            option("project_path", &["--project"], json!({"param_type": "String"})),
            dry
        ]);
        let g = detect_globals(&root);
        assert_eq!(g.json_flag.as_deref(), Some("--json"));
        assert_eq!(g.project_flag.as_deref(), Some("--project"));
        assert_eq!(g.dry_run_flag.as_deref(), Some("--dry-run"));
        let (schema, args) = click_params_to_schema(&root, &cli_anything_globals()).unwrap();
        assert_eq!(schema["properties"], json!({}));
        assert!(
            args.is_empty(),
            "--json/--project/--dry-run must not map to args: {args:?}"
        );
        // Without globals declared they would be ordinary params.
        let (schema, _) = click_params_to_schema(&root, &CliAnythingGlobals::default()).unwrap();
        assert!(schema["properties"].get("project_path").is_some());
    }

    #[test]
    fn help_and_hidden_are_skipped_and_non_arrays_rejected() {
        let mut help = option("_click_default_help", &["--help"], json!({"param_type": "Bool"}));
        help["is_flag"] = json!(true);
        let mut hidden = option("secret", &["--secret"], json!({"param_type": "String"}));
        hidden["hidden"] = json!(true);
        let (schema, args) = click_params_to_schema(&json!([help, hidden]), &CliAnythingGlobals::default()).unwrap();
        assert_eq!(schema["properties"], json!({}));
        assert!(args.is_empty());
        assert!(click_params_to_schema(&json!({}), &CliAnythingGlobals::default()).is_err());
    }
}
