const UNTRUSTED_PREFIX: &str = "[untrusted tool metadata] ";

/// Produces a deterministic, bounded summary for S1 from an untrusted remote description.
///
/// Control and known zero-width formatting characters are removed. The output always identifies
/// itself as untrusted metadata and is truncated by Unicode scalar value count.
#[must_use]
pub fn prune_summary(description: &str, max_chars: usize) -> String {
    let mut output = String::new();
    if max_chars == 0 {
        return output;
    }
    let mut character_count = 0_usize;
    let mut in_html_tag = false;
    let prefix_characters = UNTRUSTED_PREFIX.chars().count();
    for (index, character) in UNTRUSTED_PREFIX
        .chars()
        .chain(description.chars())
        .enumerate()
    {
        if character_count >= max_chars {
            break;
        }
        let from_description = index >= prefix_characters;
        if from_description && character == '<' {
            in_html_tag = true;
            continue;
        }
        if from_description && character == '>' && in_html_tag {
            in_html_tag = false;
            continue;
        }
        if from_description && (in_html_tag || is_markdown_delimiter(character)) {
            continue;
        }
        if !character.is_control() && !is_zero_width(character) {
            output.push(character);
            character_count = match character_count.checked_add(1) {
                Some(next_count) => next_count,
                None => return output,
            };
        }
    }
    output
}

fn is_markdown_delimiter(character: char) -> bool {
    matches!(
        character,
        '#' | '*' | '_' | '`' | '>' | '[' | ']' | '(' | ')' | '~'
    )
}

fn is_zero_width(character: char) -> bool {
    matches!(
        u32::from(character),
        0x00AD | 0x034F | 0x061C | 0x115F | 0x1160 | 0x17B4 | 0x17B5 | 0x180B..=0x180F
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0x3164
            | 0xFE00..=0xFE0F
            | 0xFEFF
            | 0xFFA0
            | 0xE0100..=0xE01EF
    )
}
