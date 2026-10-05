/// The words a text element holds, with an encoding this reader cannot spell
/// left as the characters it happens to decode to rather than as a file it
/// refuses: a name is worth showing however it was written, and a file with a
/// badly coded one still plays.
fn lenient_text<R: Read + Seek>(r: &mut R, e: Element, max: usize) -> Option<String> {
    Some(
        String::from_utf8_lossy(&bytes(r, e, max).ok()?)
            .trim_end_matches('\0')
            .to_owned(),
    )
}
/// The children of a master up to the first one this reader cannot read, which
/// `fields` would take as the whole list being bad. A `Tags` master is where
/// this is needed: some muxers rewrite it in place and leave a byte of padding
/// behind the last tag, and the tags standing before it are still the file's.
fn fields_to_first_gap<R: Read + Seek>(
    r: &mut R,
    e: Element,
    count: &mut usize,
    max: usize,
) -> Vec<Element> {
    let Ok(limit) = end(e) else {
        return Vec::new();
    };
    let mut at = e.data;
    let mut out = Vec::new();
    while at < limit {
        if r.seek(SeekFrom::Start(at)).is_err() {
            break;
        }
        let Ok(child) = element(r, limit, count, max) else {
            break;
        };
        let Ok(next) = end(child) else {
            break;
        };
        out.push(child);
        at = next;
    }
    out
}
/// What the whole file says about itself among its `Tags`: the `SimpleTag`
/// values of a `Tag` whose `Targets` single out no track, edition, chapter or
/// attachment. A list this reader cannot walk, or one that names nothing of the
/// kind, leaves the file stating nothing instead of failing it — as does a tag
/// of a name this player has no line for.
fn read_tags_collect<R: Read + Seek>(
    r: &mut R,
    e: Element,
    count: &mut usize,
    max: usize,
    out: &mut FileTags,
    collect: &mut impl FnMut(Option<u64>, &str, &str),
) -> bool {
    let mut complete = true;
    for tag in fields_to_first_gap(r, e, count, max) {
        if tag.id != 0x7373 {
            continue;
        }
        let Ok(parts) = fields(r, tag, count, max) else {
            complete = false;
            continue;
        };
        // Either order the two parts come in is settled before the tag is kept,
        // so a `SimpleTag` written before its `Targets` still has them.
        let mut names_a_part = false;
        let mut track_uid = None;
        let mut unsupported_scope = false;
        let mut stated: Vec<(String, String)> = Vec::new();
        for part in parts {
            match part.id {
                0x63c0 => {
                    let entries = match fields(r, part, count, max) {
                        Ok(entries) => entries,
                        Err(_) => {
                            complete = false;
                            continue;
                        }
                    };
                    for entry in entries {
                        match entry.id {
                            0x63c5 => {
                                names_a_part = true;
                                match uint(r, entry) {
                                    Ok(uid) if track_uid.is_none() => track_uid = Some(uid),
                                    _ => unsupported_scope = true,
                                }
                            }
                            0x63c4 | 0x63c6 | 0x63c9 => {
                                names_a_part = true;
                                unsupported_scope = true;
                            }
                            _ => {}
                        }
                    }
                }
                0x67c8 => {
                    let Ok(fields_of_tag) = fields(r, part, count, max) else {
                        complete = false;
                        continue;
                    };
                    let (mut name, mut value) = (String::new(), String::new());
                    for field in fields_of_tag {
                        match field.id {
                            0x45a3 => match lenient_text(r, field, 128) {
                                Some(text) => name = text,
                                None => complete = false,
                            },
                            0x4487 => match lenient_text(r, field, 1024) {
                                Some(text) => value = text,
                                None => complete = false,
                            },
                            0x67c8 => complete = false,
                            _ => {}
                        }
                    }
                    if !name.is_empty() && !value.is_empty() {
                        stated.push((name, value));
                    }
                }
                _ => {}
            }
        }
        if unsupported_scope {
            complete = false;
        }
        if !names_a_part || (track_uid.is_some() && !unsupported_scope) {
            for (name, value) in stated {
                if !names_a_part {
                    out.insert(&name, &value);
                }
                collect(track_uid, &name, &value);
            }
        }
    }
    complete
}

/// The chapter atoms of one `Chapters` master, kept in the units the file's
/// `TimestampScale` states so the scale — which the `Info` may still be to come
/// by — can be applied later. A list this reader cannot walk leaves no
/// chapters at all rather than failing the file, which plays fine without them.
fn read_chapters<R: Read + Seek>(
    r: &mut R,
    e: Element,
    count: &mut usize,
    max: usize,
    out: &mut Vec<(u64, Option<u64>, String)>,
) {
    // A master may carry no chapters at all rather than a bad one, so an
    // unknown-sized parent or a truncated list simply yields nothing.
    let Ok(editions) = fields(r, e, count, max) else {
        return;
    };
    for edition in editions {
        if edition.id != 0x45b9 {
            continue;
        }
        let Ok(atoms) = fields(r, edition, count, max) else {
            return;
        };
        for atom in atoms {
            if atom.id != 0xb6 {
                continue;
            }
            if out.len() >= 1_024 {
                return;
            }
            let Ok(entries) = fields(r, atom, count, max) else {
                return;
            };
            let mut start = None;
            let mut end = None;
            let mut title = String::new();
            for field in entries {
                match field.id {
                    0x91 => start = uint(r, field).ok(),
                    0x92 => end = uint(r, field).ok(),
                    // A chapter may be displayed in several languages; what the
                    // file leads with is what a player has to show.
                    0x80 if title.is_empty() => {
                        if let Ok(texts) = fields(r, field, count, max) {
                            for text in texts {
                                if text.id != 0x85 {
                                    continue;
                                }
                                if let Ok(bytes) = bytes(r, text, 1024) {
                                    title = String::from_utf8_lossy(&bytes)
                                        .trim_end_matches('\0')
                                        .to_owned();
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Some(start) = start {
                out.push((start, end, title));
            }
        }
    }
}
