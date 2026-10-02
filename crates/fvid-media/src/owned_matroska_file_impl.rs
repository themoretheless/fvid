fn file_metadata(value: &FileMetadata) -> Result<Vec<u8>> {
    let tags = &value.tags;
    let mut simple = Vec::new();
    for (name, value) in [
        ("TITLE", &tags.title),
        ("ARTIST", &tags.artist),
        ("ALBUM", &tags.album),
        ("GENRE", &tags.genre),
        ("DATE", &tags.date),
        ("COMMENT", &tags.comment),
        ("PART_NUMBER", &tags.track),
        ("ALBUM_ARTIST", &tags.album_artist),
        ("DISCNUMBER", &tags.disc),
        ("PUBLISHER", &tags.publisher),
        ("COPYRIGHT", &tags.copyright),
        ("DESCRIPTION", &tags.description),
        ("RATING", &tags.rating),
    ] {
        if value.contains('\0') {
            return Err(invalid("NUL in Matroska tag"));
        }
        if !value.is_empty() {
            simple.extend(element(
                0x67c8,
                &[
                    element(0x45a3, name.as_bytes())?,
                    element(0x4487, value.as_bytes())?,
                ]
                .concat(),
            )?);
        }
    }
    let mut out = Vec::new();
    if !simple.is_empty() {
        let mut tag = element(0x63c0, &[])?; // No target UID: the whole file.
        tag.extend(simple);
        out.extend(element(0x1254c367, &element(0x7373, &tag)?)?);
    }
    let mut atoms = Vec::new();
    for (i, chapter) in value.chapters.iter().enumerate() {
        if chapter.end_ns.is_some_and(|end| end < chapter.start_ns) || chapter.title.contains('\0')
        {
            return Err(invalid("invalid Matroska chapter"));
        }
        let mut atom = uint(0x73c4, i as u64 + 1)?;
        atom.extend(uint(0x91, chapter.start_ns)?);
        if let Some(end) = chapter.end_ns {
            atom.extend(uint(0x92, end)?);
        }
        if !chapter.title.is_empty() {
            atom.extend(element(
                0x80,
                &[
                    element(0x85, chapter.title.as_bytes())?,
                    element(0x437c, b"und")?,
                ]
                .concat(),
            )?);
        }
        atoms.extend(element(0xb6, &atom)?);
    }
    if !atoms.is_empty() {
        out.extend(element(0x1043a770, &element(0x45b9, &atoms)?)?);
    }
    Ok(out)
}
