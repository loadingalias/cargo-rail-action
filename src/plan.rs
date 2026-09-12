use std::collections::{BTreeSet, HashSet};
use std::fs::File;
use std::io::Read as _;
use std::path::Path;
use std::process::Command;

use rscrypto::Sha256;
use serde::de::{self, Deserialize as _, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

use crate::{ActionError, PlanOperation, Result, github, repository, write_stdout};

pub(crate) const MAX_PLAN_BYTES: usize = 64 * 1024 * 1024;
const MAX_SUBPROCESS_BYTES: usize = 1024 * 1024;
const MAX_SELECTOR_BYTES: usize = 256 * 1024;

#[derive(Debug)]
pub(crate) struct ValidatedPlan {
    bytes: Vec<u8>,
    value: Value,
}

#[derive(Debug)]
struct UniqueValue(Value);

impl<'de> serde::Deserialize<'de> for UniqueValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct UniqueVisitor;

        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON value without duplicate object keys")
            }

            fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(Number::from(value))))
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(Number::from(value))))
            }

            fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                Number::from_f64(value)
                    .map(Value::Number)
                    .map(UniqueValue)
                    .ok_or_else(|| E::custom("JSON number is not finite"))
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_string(value.to_string())
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value)))
            }

            fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }

            fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }

            fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<UniqueValue>()? {
                    values.push(value.0);
                }
                Ok(UniqueValue(Value::Array(values)))
            }

            fn visit_map<A>(self, mut object: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = Map::new();
                while let Some((key, value)) = object.next_entry::<String, UniqueValue>()? {
                    if values.insert(key.clone(), value.0).is_some() {
                        return Err(de::Error::custom(format!("duplicate JSON object key {key:?}")));
                    }
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }

        deserializer.deserialize_any(UniqueVisitor)
    }
}

pub(crate) fn parse_unique_json(bytes: &[u8], subject: &str) -> Result<Value> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let parsed = UniqueValue::deserialize(&mut deserializer)
        .map_err(|error| ActionError::rejected(format!("{subject} is invalid JSON: {error}")))?;
    deserializer
        .end()
        .map_err(|error| ActionError::rejected(format!("{subject} has trailing data: {error}")))?;
    Ok(parsed.0)
}

impl ValidatedPlan {
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| ActionError::operational(format!("cannot inspect plan '{}': {error}", path.display())))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(ActionError::rejected("plan must be a regular non-symbolic file"));
        }
        if metadata.len() > MAX_PLAN_BYTES as u64 {
            return Err(ActionError::rejected("plan exceeds the 64 MiB consumer bound"));
        }
        let capacity = usize::try_from(metadata.len())
            .map_err(|_| ActionError::rejected("plan size cannot be represented on this host"))?;
        let mut bytes = Vec::with_capacity(capacity);
        File::open(path)
            .and_then(|file| file.take(MAX_PLAN_BYTES as u64 + 1).read_to_end(&mut bytes))
            .map_err(|error| ActionError::operational(format!("cannot read plan '{}': {error}", path.display())))?;
        if bytes.len() > MAX_PLAN_BYTES {
            return Err(ActionError::rejected("plan exceeds the 64 MiB consumer bound"));
        }
        Self::from_bytes(bytes)
    }

    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        let value = parse_unique_json(&bytes, "plan")?;
        validate_plan(&value)?;
        Ok(Self { bytes, value })
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn required(&self) -> &[Value] {
        self.value["required"].as_array().expect("validated required array")
    }

    pub(crate) fn required_strings(&self) -> Vec<&str> {
        self.required()
            .iter()
            .map(|value| value.as_str().expect("validated work ID"))
            .collect()
    }

    pub(crate) fn changed_files(&self) -> usize {
        self.value["changes"]["files"]
            .as_array()
            .expect("validated file changes")
            .len()
    }

    pub(crate) fn work_count(&self) -> usize {
        self.value["work"].as_object().expect("validated work object").len()
    }

    fn decision(&self, work: &str) -> Result<&Map<String, Value>> {
        if !valid_id(work) {
            return Err(ActionError::rejected("work ID is malformed"));
        }
        self.value["work"]
            .as_object()
            .and_then(|work_map| work_map.get(work))
            .and_then(Value::as_object)
            .ok_or_else(|| ActionError::rejected(format!("plan does not register work {work}")))
    }

    fn selector_output(&self, operation: &PlanOperation) -> Result<Vec<u8>> {
        match operation {
            PlanOperation::Summary(_) => {
                let summary = self.render_summary();
                require(
                    summary.len() <= MAX_SELECTOR_BYTES,
                    "plan summary exceeds the 256 KiB display bound",
                )?;
                Ok(summary.into_bytes())
            }
            PlanOperation::Required(_) => compact_line(&self.value["required"]),
            PlanOperation::IsRequired(arguments) => {
                let state = required_string(self.decision(&arguments.work)?, "state", "work decision")?;
                Ok(format!("{}\n", state == "required").into_bytes())
            }
            PlanOperation::CargoArgs(arguments) => nul_values(self.cargo_args(&arguments.work)?),
            PlanOperation::CargoScope(arguments) => {
                let decision = self.decision(&arguments.work)?;
                if decision["state"] == "skipped" {
                    return Ok(b"skipped\n".to_vec());
                }
                let scope = object_field(decision, "scope", "work decision")?;
                require(
                    scope["kind"] == "cargo",
                    format!("work {} does not have Cargo scope", arguments.work),
                )?;
                let selection = object_field(scope, "selection", "Cargo scope")?;
                let kind = required_string(selection, "kind", "Cargo selection")?;
                Ok(format!("{kind}\n").into_bytes())
            }
            PlanOperation::PackageNames(arguments) => nul_values(self.package_names(&arguments.work)?),
            PlanOperation::TargetArgs(arguments) => nul_values(self.target_args(&arguments.work)?),
            PlanOperation::Matrix(arguments) => {
                if let Some(family) = &arguments.family
                    && !valid_id(family)
                {
                    return Err(ActionError::rejected("matrix family is malformed"));
                }
                let value = self.matrix(&arguments.work, arguments.family.as_deref())?;
                if value == "all" {
                    Ok(b"all\n".to_vec())
                } else {
                    compact_line(&value)
                }
            }
        }
    }

    fn cargo_args(&self, work: &str) -> Result<Vec<String>> {
        let decision = self.decision(work)?;
        if decision["state"] == "skipped" {
            return Ok(Vec::new());
        }
        let scope = object_field(decision, "scope", "work decision")?;
        require(
            scope["kind"] == "cargo",
            format!("work {work} does not have Cargo scope"),
        )?;
        let selection = object_field(scope, "selection", "Cargo scope")?;
        string_list(
            &selection["cargo_args"],
            &format!("work {work} Cargo argv"),
            false,
            false,
        )
    }

    fn package_names(&self, work: &str) -> Result<Vec<String>> {
        let decision = self.decision(work)?;
        if decision["state"] == "skipped" {
            return Ok(Vec::new());
        }
        let scope = object_field(decision, "scope", "work decision")?;
        require(
            scope["kind"] == "cargo",
            format!("work {work} does not have Cargo scope"),
        )?;
        let selection = object_field(scope, "selection", "Cargo scope")?;
        if selection["kind"] == "workspace" {
            return Ok(Vec::new());
        }
        let packages = selection["packages"]
            .as_array()
            .ok_or_else(|| ActionError::rejected(format!("work {work} packages are malformed")))?;
        let mut names = Vec::with_capacity(packages.len());
        for package in packages {
            let package = package
                .as_object()
                .ok_or_else(|| ActionError::rejected(format!("work {work} package is malformed")))?;
            names.push(required_string(package, "name", "package")?.to_string());
        }
        let unique = names.iter().collect::<HashSet<_>>();
        require(
            unique.len() == names.len(),
            format!("work {work} package names are ambiguous"),
        )?;
        Ok(names)
    }

    fn target_args(&self, work: &str) -> Result<Vec<String>> {
        let decision = self.decision(work)?;
        if decision["state"] == "skipped" {
            return Ok(Vec::new());
        }
        let scope = object_field(decision, "scope", "work decision")?;
        require(
            scope["kind"] == "cargo",
            format!("work {work} does not have Cargo scope"),
        )?;
        let selection = object_field(scope, "selection", "Cargo scope")?;
        let targets = selection["targets"]
            .as_array()
            .ok_or_else(|| ActionError::rejected(format!("work {work} targets are malformed")))?;
        let mut names = BTreeSet::new();
        for target in targets {
            let target = target
                .as_object()
                .ok_or_else(|| ActionError::rejected(format!("work {work} target is malformed")))?;
            let kinds = string_list(&target["kind"], "target kinds", false, true)?;
            if kinds.iter().any(|kind| kind == "test") {
                names.insert(required_string(target, "name", "target")?.to_string());
            }
        }
        Ok(names
            .into_iter()
            .flat_map(|name| ["--test".to_string(), name])
            .collect())
    }

    fn matrix(&self, work: &str, family: Option<&str>) -> Result<Value> {
        let decision = self.decision(work)?;
        if decision["state"] == "skipped" {
            return Ok(serde_json::json!({ "include": [] }));
        }
        let scope = object_field(decision, "scope", "work decision")?;
        require(
            scope["kind"] == "variants",
            format!("work {work} does not have variant scope"),
        )?;
        let selection = object_field(scope, "selection", "variant scope")?;
        if selection["kind"] == "all" {
            return Ok(Value::String("all".to_string()));
        }
        let variants = selection["variants"]
            .as_array()
            .ok_or_else(|| ActionError::rejected(format!("work {work} selected variants are malformed")))?;
        let mut rows = Vec::new();
        for variant in variants {
            let variant = variant.as_object().expect("validated variant");
            let mut dimensions = variant["dimensions"].as_object().expect("validated dimensions").clone();
            let row_family = dimensions.remove("family");
            if family.is_some() && row_family.as_ref().and_then(Value::as_str) != family {
                continue;
            }
            dimensions.insert("id".to_string(), variant["id"].clone());
            let row = Value::Object(dimensions);
            if let Some(family_name) = family {
                let mut wrapped = Map::new();
                wrapped.insert(family_name.to_string(), row);
                rows.push(Value::Object(wrapped));
            } else {
                rows.push(row);
            }
        }
        if family.is_none() && rows.is_empty() {
            return Err(ActionError::rejected(format!("work {work} selected no variants")));
        }
        Ok(serde_json::json!({ "include": rows }))
    }

    pub(crate) fn render_summary(&self) -> String {
        let required = self.required_strings();
        let skipped = self.work_count().saturating_sub(required.len());
        let changed = self.changed_files();
        let mut lines = vec![
            "## Cargo-Rail plan".to_string(),
            String::new(),
            format!(
                "{} required · {skipped} skipped · {changed} changed file{}",
                required.len(),
                if changed == 1 { "" } else { "s" }
            ),
            String::new(),
        ];
        let mut dependent_work = Vec::new();
        for id in &required {
            let decision = self.value["work"][id].as_object().expect("validated decision");
            let attribution = &self.value["attribution"][id];
            let inputs = attribution["inputs"].as_array().expect("validated inputs");
            let indirect = decision["cause"] == "changed_input"
                && !inputs.is_empty()
                && inputs
                    .iter()
                    .all(|input| matches!(input["kind"].as_str(), Some("work" | "prerequisite" | "dependency")));
            let line = format!("- `{}` — {}", id, attributed_scope(decision, attribution));
            if indirect {
                dependent_work.push(line);
            } else {
                lines.push(line);
            }
            if decision["cause"] == "forced_all" {
                lines.push("  Required by `--all`.".to_string());
            }
            if decision["cause"] == "incomplete_evidence" {
                for reference in decision["evidence"].as_array().expect("evidence") {
                    let evidence = &self.value["evidence"][reference.as_str().expect("reference")];
                    if evidence["complete"] == false {
                        lines.push(format!(
                            "  **Scope expanded:** {}",
                            github::markdown_inline(evidence["description"].as_str().expect("description"))
                        ));
                    }
                }
            }
        }
        if required.is_empty() {
            lines.push("No work required.".to_string());
        }
        if !dependent_work.is_empty() {
            lines.push(format!(
                "\n<details><summary>Additional required work: {} — dependencies</summary>\n",
                dependent_work.len()
            ));
            lines.extend(dependent_work);
            lines.push("\n</details>".to_string());
        }
        let mut dependent_selections = Vec::new();
        for id in &required {
            let attribution = &self.value["attribution"][id];
            for selection in attribution["selections"].as_array().expect("selections") {
                if selection["relation"] == "dependency" {
                    dependent_selections.push(format!(
                        "- `{}`: {} — affected by {}",
                        id,
                        github::markdown_inline(selection["subject"].as_str().expect("subject")),
                        github::markdown_inline(selection["origin"].as_str().expect("origin"))
                    ));
                }
            }
        }
        if !dependent_selections.is_empty() {
            lines.push(format!(
                "\n<details><summary>Dependent selections: {} — included in required work</summary>\n",
                dependent_selections.len()
            ));
            lines.extend(dependent_selections);
            lines.push("\n</details>".to_string());
        }
        if !required.is_empty() || skipped > 0 {
            lines.push("\n<details><summary>Work decisions</summary>\n".to_string());
            for (id, decision) in self.value["work"].as_object().expect("work") {
                lines.push(format!("- `{}` — {}", id, decision["state"].as_str().expect("state")));
                for reference in decision["evidence"].as_array().expect("evidence") {
                    let evidence = &self.value["evidence"][reference.as_str().expect("reference")];
                    lines.push(format!(
                        "  - {}",
                        github::markdown_inline(evidence["description"].as_str().expect("description"))
                    ));
                }
                if let Some(inputs) = self.value["attribution"][id]["inputs"].as_array() {
                    for input in inputs.iter().filter(|input| input["kind"] != "path") {
                        lines.push(format!(
                            "  - {}: {}",
                            input["kind"].as_str().expect("kind"),
                            github::markdown_inline(input["value"].as_str().expect("input"))
                        ));
                    }
                }
            }
            lines.push("\n</details>".to_string());
        }
        lines.push(String::new());
        lines.join("\n")
    }
}

pub(crate) fn run_command(operation: PlanOperation) -> Result<()> {
    let path = match &operation {
        PlanOperation::Required(arguments) | PlanOperation::Summary(arguments) => &arguments.plan,
        PlanOperation::IsRequired(arguments)
        | PlanOperation::CargoArgs(arguments)
        | PlanOperation::CargoScope(arguments)
        | PlanOperation::PackageNames(arguments)
        | PlanOperation::TargetArgs(arguments) => &arguments.plan,
        PlanOperation::Matrix(arguments) => &arguments.plan,
    };
    let plan = ValidatedPlan::load(path)?;
    let output = plan.selector_output(&operation)?;
    verify_checkout(plan.bytes())?;
    write_stdout(&output)
}

pub(crate) fn verify_checkout(plan: &[u8]) -> Result<()> {
    let binary = repository::find_executable("cargo-rail").ok_or_else(|| {
        ActionError::operational("matching cargo-rail binary is unavailable for saved-plan verification")
    })?;
    verify_checkout_with(plan, &binary, None)
}

pub(crate) fn verify_checkout_with(plan: &[u8], binary: &Path, workspace: Option<&Path>) -> Result<()> {
    let mut command = Command::new(binary);
    if let Some(workspace) = workspace {
        command.current_dir(workspace);
    }
    command.args(["rail", "plan", "--verify", "-"]);
    let result = repository::run_bounded_with_input(&mut command, plan, MAX_SUBPROCESS_BYTES, MAX_SUBPROCESS_BYTES)?;
    if !result.stdout.is_empty() {
        return Err(ActionError::rejected(
            "cargo-rail saved-plan verification emitted unexpected stdout",
        ));
    }
    if !result.status.success() {
        let detail = String::from_utf8_lossy(&result.stderr).trim().to_string();
        let suffix = if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        };
        return Err(ActionError::rejected(format!(
            "cargo-rail rejected current execution authority with exit code {}{suffix}",
            result.status.code().unwrap_or(1)
        )));
    }
    Ok(())
}

fn validate_plan(value: &Value) -> Result<()> {
    let plan = value
        .as_object()
        .ok_or_else(|| ActionError::rejected("plan must be an object"))?;
    exact_keys(
        plan,
        &[
            "plan_contract_version",
            "identity",
            "inputs",
            "changes",
            "work",
            "required",
            "evidence",
            "attribution",
        ],
        &[],
        "plan",
    )?;
    require(plan["plan_contract_version"] == 9, "plan contract version must be 9")?;
    let identity = required_string(plan, "identity", "plan")?;
    require(
        valid_hash_id(identity, "plan-v9:sha256:"),
        "plan identity is missing or malformed",
    )?;

    let inputs = object_field(plan, "inputs", "plan")?;
    validate_inputs(inputs)?;
    validate_changes(&plan["changes"])?;

    let evidence = object_field(plan, "evidence", "plan")?;
    validate_evidence(evidence)?;
    let required = string_list(&plan["required"], "plan required work", false, true)?;
    require(
        required.windows(2).all(|pair| pair[0] < pair[1]),
        "plan required work must be sorted",
    )?;

    let work = object_field(plan, "work", "plan")?;
    let mut projected = Vec::new();
    for (work_id, decision) in work {
        require(valid_id(work_id), "plan work ID is malformed")?;
        let decision = decision
            .as_object()
            .ok_or_else(|| ActionError::rejected(format!("work {work_id} decision must be an object")))?;
        let state = required_string(decision, "state", &format!("work {work_id}"))?;
        require(
            matches!(state, "required" | "skipped"),
            format!("work {work_id} has an invalid state"),
        )?;
        let fields: &[&str] = if state == "required" {
            &["state", "cause", "scope", "evidence"]
        } else {
            &["state", "evidence"]
        };
        exact_keys(decision, fields, &[], &format!("{state} work {work_id}"))?;
        let references = string_list(&decision["evidence"], &format!("work {work_id} evidence"), true, true)?;
        for reference in &references {
            let record = evidence
                .get(reference)
                .and_then(Value::as_object)
                .ok_or_else(|| ActionError::rejected(format!("work {work_id} references unknown evidence")))?;
            require(
                record["subject"] == *work_id,
                format!("work {work_id} references another work item's evidence"),
            )?;
        }
        if state == "required" {
            let cause = required_string(decision, "cause", &format!("work {work_id}"))?;
            require(
                matches!(cause, "changed_input" | "incomplete_evidence" | "forced_all"),
                format!("work {work_id} cause is malformed"),
            )?;
            let scope = object_field(decision, "scope", &format!("work {work_id}"))?;
            validate_selection(scope, work_id, evidence, &references)?;
            if cause == "incomplete_evidence" {
                require(
                    references
                        .iter()
                        .any(|reference| evidence[reference]["complete"] == false),
                    format!("work {work_id} incomplete decision has no incomplete evidence"),
                )?;
            }
            projected.push(work_id.clone());
        } else {
            require(
                references
                    .iter()
                    .all(|reference| evidence[reference]["complete"] == true),
                format!("skipped work {work_id} cites incomplete evidence"),
            )?;
        }
    }
    require(
        required == projected,
        "required work projection disagrees with work decisions",
    )?;
    validate_attribution(plan)?;

    let mut portable = Map::new();
    portable.insert(
        "plan_contract_version".to_string(),
        plan["plan_contract_version"].clone(),
    );
    portable.insert("inputs".to_string(), plan["inputs"].clone());
    portable.insert("changes".to_string(), plan["changes"].clone());
    portable.insert("work".to_string(), plan["work"].clone());
    portable.insert("required".to_string(), plan["required"].clone());
    portable.insert("attribution".to_string(), plan["attribution"].clone());
    portable.insert(
        "evidence".to_string(),
        Value::Array(evidence.keys().cloned().map(Value::String).collect()),
    );
    let encoded = serde_json::to_vec(&Value::Object(portable))
        .map_err(|error| ActionError::operational(format!("cannot encode plan identity: {error}")))?;
    let expected = format!("plan-v9:sha256:{}", hex_digest(&encoded));
    require(
        identity == expected,
        "plan identity does not match its typed portable content",
    )
}

fn validate_inputs(inputs: &Map<String, Value>) -> Result<()> {
    exact_keys(
        inputs,
        &[
            "base",
            "head",
            "head_commit",
            "capture",
            "cargo",
            "configuration",
            "toolchain",
            "target",
            "platform",
            "catalog",
            "evidence",
            "override",
        ],
        &[],
        "plan inputs",
    )?;
    for field in ["base", "head"] {
        required_string(inputs, field, "plan inputs")?;
    }
    for field in ["toolchain", "platform"] {
        require(
            !required_string(inputs, field, "plan inputs")?.is_empty(),
            format!("plan input {field} is empty"),
        )?;
    }
    let head_commit = required_string(inputs, "head_commit", "plan inputs")?;
    require(
        matches!(head_commit.len(), 40 | 64) && hex_range(head_commit, 40, 64),
        "plan input head_commit is malformed",
    )?;
    require(
        valid_version_hash(
            required_string(inputs, "cargo", "plan inputs")?,
            "resolution-universe-v",
        ),
        "plan input cargo is malformed",
    )?;
    require(
        valid_hash_id(
            required_string(inputs, "configuration", "plan inputs")?,
            "cargo-configuration-v1:sha256:",
        ),
        "plan input configuration is malformed",
    )?;
    require(
        valid_hash_id(
            required_string(inputs, "target", "plan inputs")?,
            "planning-target-v1:sha256:",
        ),
        "plan input target is malformed",
    )?;
    require(
        valid_hash_id(
            required_string(inputs, "catalog", "plan inputs")?,
            "work-catalog-v1:sha256:",
        ),
        "plan input catalog is malformed",
    )?;
    require(
        inputs["capture"].is_null() || inputs["capture"].is_string(),
        "plan capture is malformed",
    )?;
    let evidence = string_list(&inputs["evidence"], "plan evidence identities", false, true)?;
    require(
        evidence
            .iter()
            .all(|identity| valid_hash_id(identity, "planning-evidence-v1:sha256:")),
        "plan evidence identities are malformed",
    )?;
    require(
        matches!(inputs["override"].as_str(), Some("none" | "all")),
        "plan override is malformed",
    )
}

fn validate_changes(value: &Value) -> Result<()> {
    let changes = value
        .as_object()
        .ok_or_else(|| ActionError::rejected("plan changes must be an object"))?;
    exact_keys(changes, &["files", "cargo", "config"], &[], "plan changes")?;
    for change in array_field(changes, "files", "plan changes")? {
        let change = change
            .as_object()
            .ok_or_else(|| ActionError::rejected("file change must be an object"))?;
        exact_keys(
            change,
            &["path", "kind", "provenance"],
            &["relation", "before", "after"],
            "file change",
        )?;
        require(
            !required_string(change, "path", "file change")?.is_empty(),
            "file change path is malformed",
        )?;
        require(
            matches!(
                change["kind"].as_str(),
                Some("added" | "modified" | "type_changed" | "deleted")
            ),
            "file change kind is malformed",
        )?;
        let provenance = string_list(&change["provenance"], "file change provenance", false, true)?;
        require(
            provenance
                .iter()
                .all(|value| matches!(value.as_str(), "committed" | "staged" | "unstaged" | "untracked")),
            "file change provenance is unknown",
        )?;
        for optional in ["relation", "before", "after"] {
            if let Some(value) = change.get(optional) {
                require(value.is_string(), format!("file change {optional} must be a string"))?;
            }
        }
    }
    for change in array_field(changes, "cargo", "plan changes")? {
        let change = change
            .as_object()
            .ok_or_else(|| ActionError::rejected("Cargo change must be an object"))?;
        exact_keys(change, &["package", "target", "kind"], &[], "Cargo change")?;
        required_string(change, "package", "Cargo change")?;
        required_string(change, "kind", "Cargo change")?;
        require(
            change["target"].is_null() || change["target"].is_string(),
            "Cargo change target is malformed",
        )?;
    }
    for change in array_field(changes, "config", "plan changes")? {
        let change = change
            .as_object()
            .ok_or_else(|| ActionError::rejected("config change must be an object"))?;
        exact_keys(change, &["path", "before", "after"], &[], "config change")?;
        require(
            !required_string(change, "path", "config change")?.is_empty(),
            "config change path is malformed",
        )?;
    }
    Ok(())
}

fn validate_evidence(evidence: &Map<String, Value>) -> Result<()> {
    for (reference, record) in evidence {
        require(
            valid_hash_id(reference, "evidence:sha256:"),
            "plan evidence ID is malformed",
        )?;
        let record = record
            .as_object()
            .ok_or_else(|| ActionError::rejected(format!("plan evidence {reference} must be an object")))?;
        exact_keys(
            record,
            &["code", "subject", "description", "input", "complete"],
            &[],
            "plan evidence",
        )?;
        for field in ["code", "subject", "description"] {
            require(
                !required_string(record, field, "plan evidence")?.is_empty(),
                "plan evidence is malformed",
            )?;
        }
        require(
            valid_id(required_string(record, "subject", "plan evidence")?),
            "plan evidence subject is malformed",
        )?;
        require(
            record["input"].is_null() || record["input"].is_string(),
            "plan evidence input is malformed",
        )?;
        require(
            record["complete"].is_boolean(),
            "plan evidence completeness is malformed",
        )?;
        let mut portable = Map::new();
        for field in ["code", "subject", "input", "complete"] {
            portable.insert(field.to_string(), record[field].clone());
        }
        let encoded = serde_json::to_vec(&Value::Object(portable))
            .map_err(|error| ActionError::operational(format!("cannot encode evidence identity: {error}")))?;
        require(
            reference == &format!("evidence:sha256:{}", hex_digest(&encoded)),
            format!("plan evidence {reference} does not match its typed content"),
        )?;
    }
    Ok(())
}

fn validate_selection(
    scope: &Map<String, Value>,
    work_id: &str,
    evidence: &Map<String, Value>,
    references: &[String],
) -> Result<()> {
    let kind = required_string(scope, "kind", "work scope")?;
    if kind == "repository" {
        return exact_keys(scope, &["kind"], &[], &format!("work {work_id} repository scope"));
    }
    require(
        matches!(kind, "cargo" | "variants"),
        format!("work {work_id} has an unknown scope kind"),
    )?;
    exact_keys(scope, &["kind", "selection"], &[], &format!("work {work_id} scope"))?;
    let selection = object_field(scope, "selection", "work scope")?;
    let selection_kind = required_string(selection, "kind", "work selection")?;
    if kind == "cargo" {
        require(
            matches!(selection_kind, "workspace" | "packages"),
            format!("work {work_id} has an unknown Cargo selection"),
        )?;
        if selection_kind == "packages" {
            exact_keys(
                selection,
                &["kind", "packages", "cargo_args", "targets"],
                &[],
                "Cargo selection",
            )?;
        } else {
            exact_keys(selection, &["kind", "cargo_args", "targets"], &[], "Cargo selection")?;
        }
        let cargo_args = string_list(&selection["cargo_args"], "Cargo argv", false, false)?;
        let targets = selection["targets"]
            .as_array()
            .ok_or_else(|| ActionError::rejected("Cargo targets must be an array"))?;
        for target in targets {
            let target = target
                .as_object()
                .ok_or_else(|| ActionError::rejected("Cargo target must be an object"))?;
            exact_keys(target, &["package", "name", "kind"], &[], "Cargo target")?;
            require(
                !required_string(target, "package", "Cargo target")?.is_empty(),
                "Cargo target package is malformed",
            )?;
            require(
                !required_string(target, "name", "Cargo target")?.is_empty(),
                "Cargo target name is malformed",
            )?;
            let _ = string_list(&target["kind"], "Cargo target kinds", false, true)?;
        }
        if selection_kind == "packages" {
            let packages = selection["packages"]
                .as_array()
                .filter(|packages| !packages.is_empty())
                .ok_or_else(|| ActionError::rejected(format!("work {work_id} packages must not be empty")))?;
            let mut keys = Vec::new();
            let mut expected_args = Vec::new();
            for package in packages {
                let package = package
                    .as_object()
                    .ok_or_else(|| ActionError::rejected("Cargo package selector must be an object"))?;
                exact_keys(package, &["key", "name", "cargo_spec"], &[], "Cargo package selector")?;
                let key = required_string(package, "key", "Cargo package selector")?;
                require(!key.is_empty(), "Cargo package key is empty")?;
                require(
                    !required_string(package, "name", "Cargo package selector")?.is_empty(),
                    "Cargo package name is empty",
                )?;
                let spec = required_string(package, "cargo_spec", "Cargo package selector")?;
                require(
                    !spec.is_empty() && !spec.starts_with('-'),
                    "Cargo package spec is empty or option-like",
                )?;
                keys.push(key.to_string());
                expected_args.extend(["-p".to_string(), spec.to_string()]);
            }
            require(
                keys.windows(2).all(|pair| pair[0] < pair[1]),
                format!("work {work_id} packages are not canonical"),
            )?;
            require(
                cargo_args == expected_args,
                format!("work {work_id} Cargo argv disagrees with typed packages"),
            )?;
            require(
                targets.iter().all(|target| {
                    target["package"]
                        .as_str()
                        .is_some_and(|package| keys.iter().any(|key| key == package))
                }),
                format!("work {work_id} target references an unselected package"),
            )?;
        } else {
            require(
                cargo_args.is_empty() && targets.is_empty(),
                "workspace selection must not carry narrowing selectors",
            )?;
        }
        return Ok(());
    }

    require(
        matches!(selection_kind, "all" | "selected"),
        format!("work {work_id} has an unknown variant selection"),
    )?;
    if selection_kind == "all" {
        exact_keys(selection, &["kind", "evidence"], &[], "all-variant selection")?;
    } else {
        exact_keys(
            selection,
            &["kind", "variants", "evidence"],
            &[],
            "selected-variant selection",
        )?;
        let variants = selection["variants"]
            .as_array()
            .filter(|variants| !variants.is_empty())
            .ok_or_else(|| ActionError::rejected(format!("work {work_id} selected variants must not be empty")))?;
        for variant in variants {
            let variant = variant
                .as_object()
                .ok_or_else(|| ActionError::rejected("variant must be an object"))?;
            exact_keys(variant, &["id", "dimensions"], &[], "variant")?;
            require(
                valid_id(required_string(variant, "id", "variant")?),
                "variant ID is malformed",
            )?;
            let dimensions = object_field(variant, "dimensions", "variant")?;
            require(
                dimensions.values().all(|value| {
                    value.is_string() || value.is_boolean() || value.as_i64().is_some() || value.as_u64().is_some()
                }),
                "variant dimensions are malformed",
            )?;
        }
    }
    let reference = required_string(selection, "evidence", "variant selection")?;
    let record = evidence
        .get(reference)
        .and_then(Value::as_object)
        .ok_or_else(|| ActionError::rejected(format!("work {work_id} variant evidence is unknown")))?;
    require(
        references.iter().any(|candidate| candidate == reference),
        "variant evidence is absent from its decision",
    )?;
    require(
        record["subject"] == work_id,
        "variant evidence belongs to another work item",
    )
}

fn attributed_scope(decision: &Map<String, Value>, attribution: &Value) -> String {
    let scope = &decision["scope"];
    let selected = &scope["selection"];
    let names: std::collections::BTreeMap<&str, &str> = if selected["kind"] == "packages" {
        selected["packages"]
            .as_array()
            .expect("packages")
            .iter()
            .map(|package| {
                (
                    package["key"].as_str().expect("key"),
                    package["name"].as_str().expect("name"),
                )
            })
            .collect()
    } else if scope["kind"] == "variants" && selected["kind"] == "selected" {
        selected["variants"]
            .as_array()
            .expect("variants")
            .iter()
            .map(|variant| {
                let id = variant["id"].as_str().expect("id");
                (id, id)
            })
            .collect()
    } else {
        return scope_summary(decision);
    };
    let mut shown = Vec::new();
    let mut dependent = 0;
    for selection in attribution["selections"].as_array().expect("selections") {
        let subject = selection["subject"].as_str().expect("subject");
        if selection["relation"] == "dependency" {
            dependent += 1;
            continue;
        }
        let mut name = github::markdown_inline(names[subject]);
        if selection["relation"] == "unattributed" {
            name.push_str(" (attribution unavailable)");
        }
        if let Some(targets) = selected["targets"].as_array() {
            let targets = targets
                .iter()
                .filter(|target| target["package"] == subject)
                .map(|target| github::markdown_inline(target["name"].as_str().expect("target")))
                .collect::<Vec<_>>();
            if !targets.is_empty() {
                name.push_str(&format!(": {}", targets.join(", ")));
            }
        }
        shown.push(name);
    }
    if dependent > 0 {
        shown.push(format!(
            "{dependent} dependent selection{}",
            if dependent == 1 { "" } else { "s" }
        ));
    }
    shown.join(" · ")
}

fn validate_attribution(plan: &Map<String, Value>) -> Result<()> {
    let attribution = object_field(plan, "attribution", "plan")?;
    let work = object_field(plan, "work", "plan")?;
    let required = string_list(&plan["required"], "required", false, true)?;
    require(
        attribution.keys().map(String::as_str).collect::<Vec<_>>() == required,
        "plan attribution must cover exactly required work",
    )?;
    for (id, report) in attribution {
        let report = report
            .as_object()
            .ok_or_else(|| ActionError::rejected("work attribution must be an object"))?;
        exact_keys(report, &["inputs", "selections"], &[], "work attribution")?;
        let mut inputs = BTreeSet::new();
        for input in array_field(report, "inputs", "attribution")? {
            let input = input
                .as_object()
                .ok_or_else(|| ActionError::rejected("attribution input must be an object"))?;
            exact_keys(input, &["kind", "value"], &[], "attribution input")?;
            let kind = required_string(input, "kind", "attribution input")?;
            let value = required_string(input, "value", "attribution input")?;
            require(
                matches!(
                    kind,
                    "path" | "configuration" | "package" | "dependency" | "work" | "prerequisite"
                ),
                "unknown attribution input kind",
            )?;
            require(
                !value.is_empty() && inputs.insert((kind, value)),
                "attribution input is empty or duplicated",
            )?;
            if kind == "work" {
                require(
                    work.get(value).is_some_and(|item| item["state"] == "required"),
                    "attribution references non-required work",
                )?;
            }
        }
        let decision = work
            .get(id)
            .ok_or_else(|| ActionError::rejected("attribution references unknown work"))?;
        let scope = &decision["scope"]["selection"];
        let expected = if let Some(packages) = scope["packages"].as_array() {
            packages
                .iter()
                .map(|package| package["key"].as_str())
                .collect::<Option<BTreeSet<_>>>()
        } else if let Some(variants) = scope["variants"].as_array() {
            variants.iter().map(|variant| variant["id"].as_str()).collect()
        } else {
            Some(BTreeSet::new())
        }
        .ok_or_else(|| ActionError::rejected("attribution scope is malformed"))?;
        let mut actual = BTreeSet::new();
        let mut previous = None;
        for selection in array_field(report, "selections", "attribution")? {
            let selection = selection
                .as_object()
                .ok_or_else(|| ActionError::rejected("selection attribution must be an object"))?;
            exact_keys(
                selection,
                &["subject", "relation", "origin"],
                &[],
                "selection attribution",
            )?;
            let subject = required_string(selection, "subject", "selection attribution")?;
            let relation = required_string(selection, "relation", "selection attribution")?;
            require(
                matches!(relation, "direct" | "dependency" | "unattributed"),
                "unknown selection relation",
            )?;
            require(
                !subject.is_empty() && actual.insert(subject) && previous.is_none_or(|prior| prior < subject),
                "selection attribution must be unique and sorted",
            )?;
            previous = Some(subject);
            if relation == "dependency" {
                let origin = required_string(selection, "origin", "selection attribution")?;
                require(
                    !origin.is_empty() && origin != subject,
                    "dependency attribution requires a distinct origin",
                )?;
            } else {
                require(
                    selection["origin"].is_null(),
                    "only dependency attribution can name an origin",
                )?;
            }
        }
        require(
            actual == expected,
            "attribution selections must exactly cover executable scope",
        )?;
    }
    Ok(())
}

fn scope_summary(decision: &Map<String, Value>) -> String {
    let scope = decision["scope"].as_object().expect("validated scope");
    match scope["kind"].as_str().expect("validated scope kind") {
        "repository" => "repository".to_string(),
        "cargo" => {
            let selection = scope["selection"].as_object().expect("validated selection");
            if selection["kind"] == "workspace" {
                "Cargo workspace".to_string()
            } else {
                let packages = selection["packages"].as_array().expect("validated packages").len();
                let targets = selection["targets"].as_array().expect("validated targets").len();
                let target_suffix = if targets == 0 {
                    String::new()
                } else {
                    format!(", {targets} exact target{}", if targets == 1 { "" } else { "s" })
                };
                format!(
                    "{packages} package{}{target_suffix}",
                    if packages == 1 { "" } else { "s" }
                )
            }
        }
        "variants" => {
            let selection = scope["selection"].as_object().expect("validated selection");
            if selection["kind"] == "all" {
                "all declared variants".to_string()
            } else {
                let variants = selection["variants"].as_array().expect("validated variants").len();
                format!("{variants} variant{}", if variants == 1 { "" } else { "s" })
            }
        }
        _ => "unknown".to_string(),
    }
}

fn compact_line(value: &Value) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|error| ActionError::operational(format!("cannot encode selector output: {error}")))?;
    bytes.push(b'\n');
    if bytes.len() > MAX_SELECTOR_BYTES {
        return Err(ActionError::rejected("selector output exceeds the 256 KiB bound"));
    }
    Ok(bytes)
}

fn nul_values(values: Vec<String>) -> Result<Vec<u8>> {
    let capacity = values.iter().try_fold(0usize, |total, value| {
        total
            .checked_add(value.len() + 1)
            .ok_or_else(|| ActionError::rejected("selector output size overflow"))
    })?;
    if capacity > MAX_SELECTOR_BYTES {
        return Err(ActionError::rejected("selector output exceeds the 256 KiB bound"));
    }
    let mut output = Vec::with_capacity(capacity);
    for value in values {
        require(!value.as_bytes().contains(&0), "selector value contains NUL")?;
        output.extend_from_slice(value.as_bytes());
        output.push(0);
    }
    Ok(output)
}

fn exact_keys(object: &Map<String, Value>, required: &[&str], optional: &[&str], subject: &str) -> Result<()> {
    let required_set = required.iter().copied().collect::<BTreeSet<_>>();
    let optional_set = optional.iter().copied().collect::<BTreeSet<_>>();
    let keys = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let missing = required_set.difference(&keys).copied().collect::<Vec<_>>();
    let unknown = keys
        .difference(&required_set)
        .filter(|key| !optional_set.contains(**key))
        .copied()
        .collect::<Vec<_>>();
    require(missing.is_empty(), format!("{subject} is missing {missing:?}"))?;
    require(unknown.is_empty(), format!("{subject} has unknown fields {unknown:?}"))
}

fn object_field<'a>(object: &'a Map<String, Value>, field: &str, subject: &str) -> Result<&'a Map<String, Value>> {
    object
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| ActionError::rejected(format!("{subject}.{field} is missing or invalid")))
}

fn array_field<'a>(object: &'a Map<String, Value>, field: &str, subject: &str) -> Result<&'a [Value]> {
    object
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| ActionError::rejected(format!("{subject}.{field} must be an array")))
}

fn required_string<'a>(object: &'a Map<String, Value>, field: &str, subject: &str) -> Result<&'a str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ActionError::rejected(format!("{subject}.{field} is missing or invalid")))
}

fn string_list(value: &Value, subject: &str, nonempty: bool, unique: bool) -> Result<Vec<String>> {
    let values = value
        .as_array()
        .ok_or_else(|| ActionError::rejected(format!("{subject} must be an array of strings")))?;
    let result = values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| ActionError::rejected(format!("{subject} must contain only strings")))
        })
        .collect::<Result<Vec<_>>>()?;
    require(!nonempty || !result.is_empty(), format!("{subject} must not be empty"))?;
    if unique {
        let distinct = result.iter().collect::<HashSet<_>>();
        require(distinct.len() == result.len(), format!("{subject} contains duplicates"))?;
    }
    Ok(result)
}

fn require(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(ActionError::rejected(message))
    }
}

fn valid_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-'))
}

fn hex_range(value: &str, minimum: usize, maximum: usize) -> bool {
    (minimum..=maximum).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_hash_id(value: &str, prefix: &str) -> bool {
    value
        .strip_prefix(prefix)
        .is_some_and(|digest| hex_range(digest, 64, 64))
}

fn valid_version_hash(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|remainder| {
        remainder.split_once(":sha256:").is_some_and(|(version, digest)| {
            !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit()) && hex_range(digest, 64, 64)
        })
    })
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    fn evidence(subject: &str, code: &str, complete: bool) -> (String, Value) {
        let portable = serde_json::json!({
            "code": code,
            "subject": subject,
            "input": null,
            "complete": complete,
        });
        let identity = format!(
            "evidence:sha256:{}",
            hex_digest(&serde_json::to_vec(&portable).expect("encode evidence"))
        );
        (
            identity,
            serde_json::json!({
                "code": code,
                "subject": subject,
                "description": format!("Evidence for {subject}"),
                "input": null,
                "complete": complete,
            }),
        )
    }

    fn fixture() -> Value {
        let (cargo_evidence_id, cargo_evidence) = evidence("cargo.test", "changed_source", true);
        let (fmt_evidence_id, fmt_evidence) = evidence("cargo.fmt", "unchanged_input", true);
        let (miri_evidence_id, miri_evidence) = evidence("miri", "selected_variants", true);
        let mut evidence = Map::new();
        evidence.insert(cargo_evidence_id.clone(), cargo_evidence);
        evidence.insert(fmt_evidence_id.clone(), fmt_evidence);
        evidence.insert(miri_evidence_id.clone(), miri_evidence);
        let mut work = Map::new();
        work.insert(
            "cargo.fmt".to_string(),
            serde_json::json!({
                "state": "skipped",
                "evidence": [fmt_evidence_id],
            }),
        );
        work.insert(
            "cargo.test".to_string(),
            serde_json::json!({
                "state": "required",
                "cause": "changed_input",
                "scope": {
                    "kind": "cargo",
                    "selection": {
                        "kind": "packages",
                        "packages": [{"key": "demo", "name": "demo", "cargo_spec": "demo"}],
                        "cargo_args": ["-p", "demo"],
                        "targets": [{"package": "demo", "name": "integration", "kind": ["test"]}],
                    },
                },
                "evidence": [cargo_evidence_id],
            }),
        );
        work.insert(
            "miri".to_string(),
            serde_json::json!({
                "state": "required",
                "cause": "changed_input",
                "scope": {
                    "kind": "variants",
                    "selection": {
                        "kind": "selected",
                        "variants": [{"id": "linux", "dimensions": {"family": "miri", "target": "x86_64"}}],
                        "evidence": miri_evidence_id,
                    },
                },
                "evidence": [miri_evidence_id],
            }),
        );
        let mut value = serde_json::json!({
            "plan_contract_version": 9,
            "identity": "",
            "inputs": {
                "base": "base",
                "head": "head",
                "head_commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "capture": null,
                "cargo": format!("resolution-universe-v1:sha256:{}", "b".repeat(64)),
                "configuration": format!("cargo-configuration-v1:sha256:{}", "c".repeat(64)),
                "toolchain": "toolchain-v1",
                "target": format!("planning-target-v1:sha256:{}", "d".repeat(64)),
                "platform": "linux-x86_64",
                "catalog": format!("work-catalog-v1:sha256:{}", "e".repeat(64)),
                "evidence": [],
                "override": "none",
            },
            "changes": {
                "files": [{"path": "src/lib.rs", "kind": "modified", "provenance": ["committed"]}],
                "cargo": [],
                "config": [],
            },
            "work": work,
            "attribution": {
                "cargo.test": {"inputs": [{"kind": "path", "value": "src/lib.rs"}], "selections": [{"subject": "demo", "relation": "direct", "origin": null}]},
                "miri": {"inputs": [{"kind": "package", "value": "demo"}], "selections": [{"subject": "linux", "relation": "direct", "origin": null}]}
            },
            "required": ["cargo.test", "miri"],
            "evidence": evidence,
        });
        set_identity(&mut value);
        value
    }

    fn set_identity(value: &mut Value) {
        let plan = value.as_object().expect("plan object");
        let mut portable = Map::new();
        for field in [
            "plan_contract_version",
            "inputs",
            "changes",
            "work",
            "required",
            "attribution",
        ] {
            portable.insert(field.to_string(), plan[field].clone());
        }
        portable.insert(
            "evidence".to_string(),
            Value::Array(
                plan["evidence"]
                    .as_object()
                    .expect("evidence")
                    .keys()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
        value["identity"] = Value::String(format!(
            "plan-v9:sha256:{}",
            hex_digest(&serde_json::to_vec(&Value::Object(portable)).expect("portable plan"))
        ));
    }

    #[test]
    fn attribution_is_bound_and_dependency_details_do_not_hide_direct_work() {
        let mut value = fixture();
        for index in 0..60 {
            let id = format!("direct{index:02}");
            let (reference, proof) = evidence(&id, "changed_input", true);
            value["evidence"][&reference] = proof;
            let mut decision = value["work"]["cargo.test"].clone();
            decision["evidence"] = serde_json::json!([reference]);
            value["work"][&id] = decision;
            value["attribution"][&id] = value["attribution"]["cargo.test"].clone();
        }
        let (reference, proof) = evidence("dependent", "changed_input", true);
        value["evidence"][&reference] = proof;
        value["work"]["dependent"] = value["work"]["cargo.test"].clone();
        value["work"]["dependent"]["evidence"] = serde_json::json!([reference]);
        value["attribution"]["dependent"] = value["attribution"]["cargo.test"].clone();
        value["attribution"]["dependent"]["inputs"] = serde_json::json!([{"kind": "work", "value": "cargo.test"}]);
        value["required"] = Value::Array(
            value["work"]
                .as_object()
                .unwrap()
                .iter()
                .filter(|(_, item)| item["state"] == "required")
                .map(|(id, _)| Value::String(id.clone()))
                .collect(),
        );
        set_identity(&mut value);
        let plan = ValidatedPlan::from_bytes(serde_json::to_vec(&value).unwrap()).unwrap();
        let summary = plan.render_summary();
        let visible = summary.split("<details>").next().unwrap();
        assert!(visible.contains("direct59"), "direct work was truncated");
        assert!(visible.contains("demo: integration"));
        assert!(!visible.contains("`dependent`"));
        assert!(summary.contains("`dependent`"));
        value["attribution"]["cargo.test"]["selections"][0]["relation"] = "unattributed".into();
        assert!(validate_plan(&value).unwrap_err().to_string().contains("identity"));
        set_identity(&mut value);
        value["attribution"]["cargo.test"]["selections"] = serde_json::json!([]);
        assert!(validate_plan(&value).unwrap_err().to_string().contains("exactly cover"));
    }

    #[test]
    fn incomplete_scope_reason_stays_visible() {
        let mut value = fixture();
        let (reference, mut proof) = evidence("cargo.test", "compiler_inputs_incomplete", false);
        proof["description"] = "Build-script input evidence is unavailable".into();
        value["evidence"][&reference] = proof;
        value["work"]["cargo.test"]["cause"] = "incomplete_evidence".into();
        value["work"]["cargo.test"]["evidence"] = serde_json::json!([reference]);
        set_identity(&mut value);
        let plan = ValidatedPlan::from_bytes(serde_json::to_vec(&value).unwrap()).unwrap();
        let summary = plan.render_summary();
        assert!(
            summary
                .split("<details>")
                .next()
                .unwrap()
                .contains("Scope expanded:** Build-script input evidence is unavailable")
        );
    }

    #[test]
    fn duplicate_json_keys_are_rejected_at_every_depth() {
        let error =
            parse_unique_json(br#"{"outer":{"value":1,"value":2}}"#, "fixture").expect_err("duplicate key must fail");
        assert!(error.to_string().contains("duplicate"));
    }

    #[test]
    fn missing_decision_evidence_is_rejected_before_indexing() {
        for (work, state) in [("cargo.test", "required"), ("cargo.fmt", "skipped")] {
            let mut value = fixture();
            value["work"][work].as_object_mut().unwrap().remove("evidence");
            let error = ValidatedPlan::from_bytes(serde_json::to_vec(&value).unwrap())
                .expect_err("missing evidence must be rejected");
            assert_eq!(error.kind, crate::ErrorKind::Rejected);
            assert_eq!(
                error.to_string(),
                format!("{state} work {work} is missing [\"evidence\"]")
            );
        }
    }

    #[test]
    fn target_arguments_reject_non_cargo_work() {
        for scope in [
            fixture()["work"]["miri"]["scope"].clone(),
            serde_json::json!({"kind": "repository"}),
        ] {
            let mut value = fixture();
            value["work"]["miri"]["scope"] = scope;
            if value["work"]["miri"]["scope"]["kind"] == "repository" {
                value["attribution"]["miri"]["selections"] = serde_json::json!([]);
            }
            set_identity(&mut value);
            let plan = ValidatedPlan::from_bytes(serde_json::to_vec(&value).unwrap()).unwrap();
            let error = plan.target_args("miri").expect_err("non-Cargo scope");
            assert_eq!(error.kind, crate::ErrorKind::Rejected);
            assert_eq!(error.to_string(), "work miri does not have Cargo scope");
        }
    }

    #[test]
    fn head_commit_requires_a_complete_git_object_id() {
        for length in [40, 64] {
            let mut value = fixture();
            value["inputs"]["head_commit"] = "a".repeat(length).into();
            set_identity(&mut value);
            validate_plan(&value).expect("full Git object ID");
        }
        for length in [39, 41, 63, 65] {
            let mut value = fixture();
            value["inputs"]["head_commit"] = "a".repeat(length).into();
            set_identity(&mut value);
            assert_eq!(
                validate_plan(&value).unwrap_err().to_string(),
                "plan input head_commit is malformed"
            );
        }
    }

    #[test]
    fn identifiers_are_narrow() {
        assert!(valid_id("cargo.test"));
        assert!(!valid_id("-package"));
        assert!(!valid_id("Cargo.test"));
    }

    #[test]
    fn validates_and_projects_every_selector_shape() {
        let value = fixture();
        validate_plan(&value).expect("valid plan");
        let plan = ValidatedPlan::from_bytes(serde_json::to_vec(&value).expect("encode plan")).expect("validate plan");
        assert_eq!(plan.cargo_args("cargo.test").expect("cargo args"), ["-p", "demo"]);
        assert_eq!(plan.package_names("cargo.test").expect("packages"), ["demo"]);
        assert_eq!(
            plan.target_args("cargo.test").expect("targets"),
            ["--test", "integration"]
        );
        assert!(plan.cargo_args("cargo.fmt").expect("skipped args").is_empty());
        assert_eq!(
            plan.matrix("miri", Some("miri")).expect("family matrix"),
            serde_json::json!({"include": [{"miri": {"id": "linux", "target": "x86_64"}}]})
        );
        let summary = plan.render_summary();
        assert!(summary.contains("2 required · 1 skipped · 1 changed file"));
        assert!(!summary.contains("plan-v9:sha256:"));
    }

    #[test]
    fn human_description_is_outside_portable_identity() {
        let mut value = fixture();
        let reference = value["evidence"]
            .as_object()
            .expect("evidence")
            .keys()
            .next()
            .cloned()
            .expect("record");
        value["evidence"][&reference]["description"] = Value::String("Improved explanation".to_string());
        validate_plan(&value).expect("description-only change remains valid");
    }

    #[test]
    fn rejects_identity_projection_and_option_inconsistency() {
        let mut identity = fixture();
        identity["identity"] = Value::String(format!("plan-v9:sha256:{}", "0".repeat(64)));
        assert!(
            validate_plan(&identity)
                .expect_err("identity drift")
                .to_string()
                .contains("identity")
        );

        let mut projection = fixture();
        projection["required"] = serde_json::json!(["miri"]);
        set_identity(&mut projection);
        assert!(
            validate_plan(&projection)
                .expect_err("projection drift")
                .to_string()
                .contains("projection")
        );

        let mut option = fixture();
        option["work"]["cargo.test"]["scope"]["selection"]["packages"][0]["cargo_spec"] =
            Value::String("--workspace".to_string());
        option["work"]["cargo.test"]["scope"]["selection"]["cargo_args"] = serde_json::json!(["-p", "--workspace"]);
        set_identity(&mut option);
        assert!(
            validate_plan(&option)
                .expect_err("option-like package")
                .to_string()
                .contains("option-like")
        );
    }

    #[cfg(unix)]
    #[test]
    fn checkout_verification_uses_captured_bytes_and_selected_directory() {
        let directory =
            repository::create_private_directory(&std::env::temp_dir(), "cargo-rail-action-plan-authority-test")
                .expect("temporary directory");
        let plan_path = directory.join("plan.json");
        let original = serde_json::to_vec(&fixture()).expect("encode plan");
        std::fs::write(&plan_path, &original).expect("write plan");
        let plan = ValidatedPlan::load(&plan_path).expect("load plan");
        std::fs::write(&plan_path, b"replaced after capture\n").expect("replace plan path");

        let verifier = directory.join("cargo-rail");
        std::fs::write(
            &verifier,
            b"#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$0.args\"\npwd -P > \"$0.cwd\"\ncat > \"$0.stdin\"\n",
        )
        .expect("write verifier");
        std::fs::set_permissions(&verifier, std::fs::Permissions::from_mode(0o700)).expect("verifier permissions");

        let workspace = directory.join("nested workspace");
        std::fs::create_dir(&workspace).expect("nested workspace");
        for selected in [None, Some(workspace.as_path())] {
            verify_checkout_with(plan.bytes(), &verifier, selected).expect("verify captured bytes");
            let expected = selected
                .map(Path::to_path_buf)
                .unwrap_or_else(|| std::env::current_dir().expect("current directory"));
            assert_eq!(
                std::fs::read_to_string(verifier.with_extension("cwd"))
                    .expect("captured directory")
                    .trim_end(),
                std::fs::canonicalize(expected)
                    .expect("canonical directory")
                    .to_str()
                    .unwrap()
            );
        }

        assert_eq!(
            std::fs::read(verifier.with_extension("stdin")).expect("captured stdin"),
            original
        );
        assert_eq!(
            std::fs::read_to_string(verifier.with_extension("args")).expect("captured arguments"),
            "rail\nplan\n--verify\n-\n"
        );
        std::fs::remove_dir_all(directory).expect("remove fixture");
    }
}
