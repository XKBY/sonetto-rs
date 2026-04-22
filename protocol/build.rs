use std::{collections::BTreeMap, fs, path::Path};

fn parse_optional_enum_path(line: &str) -> Option<String> {
    if !line.contains("#[prost(enumeration = ") || !line.contains(", optional") {
        return None;
    }
    let marker = "enumeration = \"";
    let start = line.find(marker)? + marker.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn sanitize_enum_wrapper_name(enum_path: &str) -> String {
    enum_path
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => ch,
            _ => '_',
        })
        .collect()
}

fn normalize_enum_variant_name(name: &str) -> String {
    name.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn collect_enum_variants(generated: &str) -> BTreeMap<String, Vec<(String, String)>> {
    let mut enum_variants = BTreeMap::new();
    let mut module_stack: Vec<(String, usize)> = Vec::new();
    let mut brace_depth = 0usize;
    let lines: Vec<&str> = generated.lines().collect();

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("pub mod ") {
            if let Some(name) = rest.strip_suffix(" {") {
                module_stack.push((name.to_string(), brace_depth + 1));
            }
        }

        if let Some(rest) = trimmed.strip_prefix("pub enum ") {
            if let Some(name) = rest.strip_suffix(" {") {
                let mut variants = Vec::new();
                let mut lookahead = index + 1;
                while lookahead < lines.len() {
                    let enum_line = lines[lookahead].trim();
                    if enum_line == "}" {
                        break;
                    }
                    if let Some((variant_name, _)) = enum_line.split_once(" = ") {
                        let variant_name = variant_name.trim();
                        if !variant_name.is_empty() {
                            variants.push((
                                variant_name.to_string(),
                                normalize_enum_variant_name(variant_name),
                            ));
                        }
                    }
                    lookahead += 1;
                }

                let module_path = module_stack
                    .iter()
                    .map(|(module, _)| module.as_str())
                    .collect::<Vec<_>>()
                    .join("::");
                let full_path = if module_path.is_empty() {
                    name.to_string()
                } else {
                    format!("{module_path}::{name}")
                };
                enum_variants.insert(full_path, variants);
            }
        }

        let opens = line.chars().filter(|ch| *ch == '{').count();
        let closes = line.chars().filter(|ch| *ch == '}').count();
        brace_depth = brace_depth + opens - closes;
        while module_stack
            .last()
            .map(|(_, depth)| brace_depth < *depth)
            .unwrap_or(false)
        {
            module_stack.pop();
        }
    }

    enum_variants
}

fn build_enum_wrapper(
    enum_path: &str,
    wrapper_name: &str,
    variants: &[(String, String)],
) -> String {
    let mut out = String::new();
    out.push_str("\n#[doc(hidden)]\n");
    out.push_str("#[allow(non_snake_case)]\n");
    out.push_str(&format!(
        "pub fn {wrapper_name}<'de, D>(deserializer: D) -> Result<::core::option::Option<i32>, D::Error>\n"
    ));
    out.push_str("where\n    D: serde::de::Deserializer<'de>,\n{\n");
    out.push_str(
        "    crate::serde_helpers::option_enum_i32_from_string_or_number(deserializer, |value| {\n",
    );
    out.push_str(&format!(
        "        {enum_path}::from_str_name(value).map(|variant| variant as i32).or_else(|| {{\n"
    ));
    out.push_str("            let normalized = value\n");
    out.push_str("                .chars()\n");
    out.push_str("                .filter(|ch| ch.is_ascii_alphanumeric())\n");
    out.push_str("                .map(|ch| ch.to_ascii_lowercase())\n");
    out.push_str("                .collect::<::std::string::String>();\n");
    out.push_str("            match normalized.as_str() {\n");
    for (variant_name, normalized) in variants {
        out.push_str(&format!(
            "                \"{normalized}\" => Some({enum_path}::{variant_name} as i32),\n"
        ));
    }
    out.push_str("                _ => None,\n");
    out.push_str("            }\n");
    out.push_str("        })\n");
    out.push_str("    })\n");
    out.push_str("}\n");
    out
}

fn main() {
    println!("cargo::rustc-check-cfg=cfg(rust_analyzer)");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=src/lib.rs");

    let proto_files = ["sonetto.proto", "cmd_id.proto"];

    for proto in &proto_files {
        println!("cargo::rerun-if-changed={proto}");
    }

    if proto_files.iter().all(|f| Path::new(f).exists()) {
        prost_build::Config::new()
            .type_attribute(
                ".",
                "#[derive(serde::Serialize, serde::Deserialize)]\n#[serde(rename_all = \"camelCase\")]",
            )
            .message_attribute(".", r#"#[serde(default)]"#)
            .field_attribute("*.type", "#[serde(rename = \"type\")]")
            .out_dir("include/")
            .compile_protos(&proto_files, &["."])
            .expect("Failed to compile proto files");

        let include_path = Path::new("include").join("_.rs");
        let generated = fs::read_to_string(&include_path).expect("Failed to read generated proto");
        let generated = generated
            .replace(
                "    #[prost(int64, optional",
                "    #[serde(default, deserialize_with = \"crate::serde_helpers::option_i64_from_number_or_string\")]\n    #[prost(int64, optional",
            )
            .replace(
                "    #[prost(int64, repeated",
                "    #[serde(default, deserialize_with = \"crate::serde_helpers::vec_i64_from_number_or_string\")]\n    #[prost(int64, repeated",
            )
            .replace(
                "    #[prost(uint64, optional",
                "    #[serde(default, deserialize_with = \"crate::serde_helpers::option_u64_from_number_or_string\")]\n    #[prost(uint64, optional",
            )
            .replace(
                "    #[prost(uint64, repeated",
                "    #[serde(default, deserialize_with = \"crate::serde_helpers::vec_u64_from_number_or_string\")]\n    #[prost(uint64, repeated",
            );
        let enum_variants = collect_enum_variants(&generated);
        let mut wrapper_names = BTreeMap::<String, String>::new();
        let mut patched = String::new();

        for line in generated.lines() {
            if let Some(enum_path) = parse_optional_enum_path(line) {
                let wrapper_name = wrapper_names
                    .entry(enum_path.clone())
                    .or_insert_with(|| {
                        format!(
                            "__serde_option_enum_i32_{}",
                            sanitize_enum_wrapper_name(&enum_path)
                        )
                    })
                    .clone();
                patched.push_str(&format!(
                    "    #[serde(default, deserialize_with = \"crate::{wrapper_name}\")]\n"
                ));
            }
            patched.push_str(line);
            patched.push('\n');
        }

        for (enum_path, wrapper_name) in &wrapper_names {
            let variants = enum_variants
                .get(enum_path)
                .unwrap_or_else(|| panic!("Missing enum definition for {enum_path}"));
            patched.push_str(&build_enum_wrapper(enum_path, wrapper_name, variants));
        }

        fs::write(&include_path, patched).expect("Failed to patch generated proto serde");
    }
}
