use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use uuid::Uuid;

/// A script module the pack depends on, such as `@minecraft/server`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptModule {
    pub name: String,
    pub version: String,
}

/// The UUID of the pack's script module.
///
/// It's derived from the header UUID and the package name, so it stays the same across builds
/// and differs between packs.
pub fn script_uuid(header_uuid: Uuid, package_name: &str) -> Uuid {
    Uuid::new_v5(&header_uuid, package_name.as_bytes())
}

/// Adds the native script module with `entry` to `manifest`, and a dependency on each of
/// `dependencies`.
///
/// An existing native script module is replaced, as is an existing dependency on the same module.
/// Other modules and dependencies are kept.
pub fn patch(
    manifest: &mut Value,
    entry: &str,
    package_name: &str,
    dependencies: &[ScriptModule],
) -> Result<()> {
    let root = manifest
        .as_object_mut()
        .context("the manifest isn't a JSON object")?;
    let header = &root["header"];
    let header_uuid = header["uuid"]
        .as_str()
        .context("the manifest has no header.uuid")?;
    let header_uuid = Uuid::parse_str(header_uuid)
        .with_context(|| format!("header.uuid `{header_uuid}` isn't a UUID"))?;
    let version = header
        .get("version")
        .cloned()
        .context("the manifest has no header.version")?;

    let script = json!({
        "type": "script",
        "language": "native",
        "entry": entry,
        "uuid": script_uuid(header_uuid, package_name).to_string(),
        "version": version,
    });
    let modules = array(root, "modules")?;
    let mut existing = None;
    for (index, module) in modules.iter().enumerate() {
        if module["type"] != "script" {
            continue;
        }
        if module["language"] != "native" {
            let language = module["language"].as_str().unwrap_or("javascript");
            bail!(
                "the manifest already has a {language} script module. A pack can have only one \
                 script module, so remove it to use a native one"
            );
        }
        existing = Some(index);
    }
    match existing {
        Some(index) => modules[index] = script,
        None => modules.push(script),
    }

    let declared = array(root, "dependencies")?;
    for dependency in dependencies {
        let value = json!({ "module_name": dependency.name, "version": dependency.version });
        match declared
            .iter_mut()
            .find(|declared| declared["module_name"] == *dependency.name)
        {
            Some(declared) => *declared = value,
            None => declared.push(value),
        }
    }
    Ok(())
}

/// The array at `key`, created if it's missing.
fn array<'a>(object: &'a mut Map<String, Value>, key: &str) -> Result<&'a mut Vec<Value>> {
    object
        .entry(key)
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .with_context(|| format!("the manifest's {key} isn't an array"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER_UUID: &str = "a356d1d8-f722-4d77-9e27-503cd2069714";

    fn server() -> ScriptModule {
        ScriptModule {
            name: "@minecraft/server".to_owned(),
            version: "2.11.0".to_owned(),
        }
    }

    fn manifest(modules: &Value, dependencies: &Value) -> Value {
        json!({
            "format_version": 2,
            "header": {
                "name": "Hello",
                "description": "",
                "uuid": HEADER_UUID,
                "version": [1, 2, 3],
                "min_engine_version": [1, 26, 60]
            },
            "modules": modules,
            "dependencies": dependencies
        })
    }

    fn expected_uuid() -> String {
        script_uuid(Uuid::parse_str(HEADER_UUID).unwrap(), "hello-world").to_string()
    }

    #[test]
    fn uuid_is_stable() {
        let header = Uuid::parse_str(HEADER_UUID).unwrap();
        assert_eq!(
            script_uuid(header, "hello-world"),
            script_uuid(header, "hello-world")
        );
        assert_ne!(
            script_uuid(header, "hello-world"),
            script_uuid(header, "shape-check")
        );
        // Computed with Python's `uuid.uuid5`. Changing the derivation would change the UUID of
        // every published pack.
        assert_eq!(
            script_uuid(header, "hello-world").to_string(),
            "3ad139ca-ee88-52fd-81b2-c36a9a35b003"
        );
    }

    #[test]
    fn adds_module_and_dependency() {
        let data = json!({ "type": "data", "uuid": "7bce44d2-4cbd-418a-be8c-3b41fc40a7d8", "version": [1, 0, 0] });
        let mut value = manifest(&json!([data]), &json!([]));
        patch(
            &mut value,
            "scripts/hello_world.dll",
            "hello-world",
            &[server()],
        )
        .unwrap();
        assert_eq!(
            value["modules"],
            json!([
                data,
                {
                    "type": "script",
                    "language": "native",
                    "entry": "scripts/hello_world.dll",
                    "uuid": expected_uuid(),
                    "version": [1, 2, 3]
                }
            ])
        );
        assert_eq!(
            value["dependencies"],
            json!([{ "module_name": "@minecraft/server", "version": "2.11.0" }])
        );
    }

    #[test]
    fn creates_missing_arrays() {
        let mut value = manifest(&json!([]), &json!([]));
        let object = value.as_object_mut().unwrap();
        object.remove("modules");
        object.remove("dependencies");
        patch(
            &mut value,
            "scripts/hello_world.dll",
            "hello-world",
            &[server()],
        )
        .unwrap();
        assert_eq!(value["modules"].as_array().unwrap().len(), 1);
        assert_eq!(value["dependencies"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn replaces_native_module() {
        let old = json!({ "type": "script", "language": "native", "entry": "scripts/old.dll", "uuid": "00000000-0000-0000-0000-000000000000", "version": [0, 1, 0] });
        let mut value = manifest(&json!([old]), &json!([]));
        patch(&mut value, "scripts/hello_world.dll", "hello-world", &[]).unwrap();
        let modules = value["modules"].as_array().unwrap();
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0]["entry"], "scripts/hello_world.dll");
        assert_eq!(modules[0]["uuid"], expected_uuid());
    }

    #[test]
    fn refuses_javascript_module() {
        let js = json!({ "type": "script", "language": "javascript", "entry": "scripts/main.js", "uuid": "f29456a4-7a01-4840-97ff-fbf8860d0086", "version": [1, 0, 0] });
        let mut value = manifest(&json!([js]), &json!([]));
        let error = patch(&mut value, "scripts/a.dll", "a", &[]).unwrap_err();
        assert!(error.to_string().contains("javascript script module"));
    }

    #[test]
    fn refuses_script_module_without_language() {
        let js = json!({ "type": "script", "entry": "scripts/main.js", "uuid": "f29456a4-7a01-4840-97ff-fbf8860d0086", "version": [1, 0, 0] });
        let mut value = manifest(&json!([js]), &json!([]));
        assert!(patch(&mut value, "scripts/a.dll", "a", &[]).is_err());
    }

    #[test]
    fn replaces_dependency_and_keeps_others() {
        let mut value = manifest(
            &json!([]),
            &json!([
                { "uuid": "266518b0-e117-4c55-9751-d73b5bf4794d", "version": [1, 0, 0] },
                { "module_name": "@minecraft/server", "version": "1.0.0" }
            ]),
        );
        patch(&mut value, "scripts/a.dll", "a", &[server()]).unwrap();
        assert_eq!(
            value["dependencies"],
            json!([
                { "uuid": "266518b0-e117-4c55-9751-d73b5bf4794d", "version": [1, 0, 0] },
                { "module_name": "@minecraft/server", "version": "2.11.0" }
            ])
        );
    }

    #[test]
    fn keeps_header_fields() {
        let mut value = manifest(&json!([]), &json!([]));
        let header = value["header"].clone();
        patch(&mut value, "scripts/a.dll", "a", &[server()]).unwrap();
        assert_eq!(value["header"], header);
        assert_eq!(value["format_version"], 2);
    }

    #[test]
    fn rejects_bad_header() {
        let mut value = manifest(&json!([]), &json!([]));
        value["header"]["uuid"] = json!("not a uuid");
        assert!(patch(&mut value, "scripts/a.dll", "a", &[]).is_err());
        let mut value = manifest(&json!([]), &json!([]));
        value["header"].as_object_mut().unwrap().remove("version");
        assert!(patch(&mut value, "scripts/a.dll", "a", &[]).is_err());
    }
}
