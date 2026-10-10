//! Restricted `SKILL.md` frontmatter parser: required `name` and
//! `description`, optional scalar `argument-hint` and `license`, plain or
//! JSON-quoted scalars, and simple two-space literal/folded description blocks.
//! This intentionally does not implement general YAML features.

use super::MAX_SKILL_DESCRIPTION_BYTES;
use super::metadata::{ParsedMetadata, SkillIndexError};

const MAX_OPTIONAL_FIELD_BYTES: usize = 8 * 1024;

pub(super) fn parse_frontmatter(bytes: &[u8]) -> Result<ParsedMetadata, SkillIndexError> {
    let source = std::str::from_utf8(bytes).map_err(|_| SkillIndexError::MalformedFrontmatter)?;
    let lines: Vec<&str> = source
        .split_terminator('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    if lines.len() < 3 || lines.first() != Some(&"---") || lines.last() != Some(&"---") {
        return Err(SkillIndexError::MalformedFrontmatter);
    }

    let mut name = None;
    let mut description = None;
    let mut seen_optional = [false; 2];
    let mut line_index = 1;
    while line_index + 1 < lines.len() {
        let line = lines[line_index];
        line_index += 1;
        if line.is_empty() {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(SkillIndexError::MalformedFrontmatter);
        }
        let (key, raw_value) = line
            .split_once(':')
            .ok_or(SkillIndexError::MalformedFrontmatter)?;
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
        {
            return Err(SkillIndexError::MalformedFrontmatter);
        }
        if !raw_value.starts_with(' ') {
            return Err(SkillIndexError::MalformedFrontmatter);
        }
        let value = raw_value.trim_start_matches(' ');
        match key {
            "name" => {
                if name.is_some() || value.is_empty() || value == "|" || value.starts_with('>') {
                    return Err(SkillIndexError::MalformedFrontmatter);
                }
                let value = parse_scalar(value)?;
                if !valid_name(&value) {
                    return Err(SkillIndexError::MalformedFrontmatter);
                }
                name = Some(value);
            }
            "description" => {
                if description.is_some() {
                    return Err(SkillIndexError::MalformedFrontmatter);
                }
                let value = if let Some(folded) = block_style(value) {
                    parse_description_block(&lines, &mut line_index, folded)?
                } else {
                    parse_scalar(value)?
                };
                if value.trim().is_empty() || value.len() > MAX_SKILL_DESCRIPTION_BYTES {
                    return Err(SkillIndexError::MalformedFrontmatter);
                }
                description = Some(value);
            }
            "argument-hint" | "license" => {
                let optional_index = if key == "license" { 1 } else { 0 };
                if seen_optional[optional_index] || value.is_empty() || block_style(value).is_some()
                {
                    return Err(SkillIndexError::MalformedFrontmatter);
                }
                let scalar = parse_scalar(value)?;
                if scalar.len() > MAX_OPTIONAL_FIELD_BYTES {
                    return Err(SkillIndexError::MalformedFrontmatter);
                }
                seen_optional[optional_index] = true;
            }
            _ => return Err(SkillIndexError::MalformedFrontmatter),
        }
    }

    Ok(ParsedMetadata {
        name: name.ok_or(SkillIndexError::MalformedFrontmatter)?,
        description: description.ok_or(SkillIndexError::MalformedFrontmatter)?,
    })
}

fn valid_name(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value.is_ascii()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn parse_scalar(value: &str) -> Result<String, SkillIndexError> {
    if value.starts_with('"') {
        let scalar: String =
            serde_json::from_str(value).map_err(|_| SkillIndexError::MalformedFrontmatter)?;
        if scalar.chars().any(char::is_control) {
            return Err(SkillIndexError::MalformedFrontmatter);
        }
        return Ok(scalar);
    }
    if value.chars().next().is_some_and(|character| {
        matches!(character, '&' | '*' | '!' | '{' | '[' | '|' | '>' | '\'')
    }) || value.contains(": ")
        || value.contains(" #")
        || value.starts_with("- ")
        || value.starts_with("? ")
        || value.chars().any(char::is_control)
    {
        return Err(SkillIndexError::MalformedFrontmatter);
    }
    Ok(value.trim().to_owned())
}

fn block_style(value: &str) -> Option<bool> {
    match value {
        "|" => Some(false),
        ">" => Some(true),
        _ => None,
    }
}

fn parse_description_block(
    lines: &[&str],
    line_index: &mut usize,
    folded: bool,
) -> Result<String, SkillIndexError> {
    let mut block = Vec::new();
    while *line_index + 1 < lines.len() {
        let line = lines[*line_index];
        if line.starts_with("  ") {
            block.push(&line[2..]);
            *line_index += 1;
        } else if line.is_empty() {
            block.push("");
            *line_index += 1;
        } else {
            break;
        }
    }
    if block.is_empty()
        || block
            .iter()
            .any(|line| line.starts_with(' ') || line.starts_with('\t'))
    {
        return Err(SkillIndexError::MalformedFrontmatter);
    }
    let mut text = if folded {
        fold_lines(&block)
    } else {
        block.join("\n")
    };
    while text.ends_with('\n') {
        text.pop();
    }
    text.push('\n');
    Ok(text)
}

fn fold_lines(lines: &[&str]) -> String {
    let mut output = String::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            if line.is_empty() || lines[index - 1].is_empty() {
                output.push('\n');
            } else {
                output.push(' ');
            }
        }
        output.push_str(line);
    }
    output
}
