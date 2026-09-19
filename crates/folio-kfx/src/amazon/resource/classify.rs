pub(crate) fn classify_resource_shape(
    format: Option<&str>,
    mime: Option<&str>,
    tiled: bool,
    overlapped_tiles: bool,
) -> String {
    if overlapped_tiles {
        return "OverlappedTile".to_owned();
    }
    if tiled {
        return "Tiled".to_owned();
    }
    let format = format.unwrap_or_default().to_ascii_lowercase();
    let mime = mime.unwrap_or_default().to_ascii_lowercase();
    if format == "pdf" || mime == "pdf" || mime == "application/pdf" {
        return "PDF".to_owned();
    }
    if format == "svg" || mime == "image/svg+xml" {
        return "Vector".to_owned();
    }
    if ["jpg", "jpeg", "png", "gif", "webp", "jxr"].contains(&format.as_str())
        || mime.starts_with("image/")
    {
        return "DirectMedia".to_owned();
    }
    "Unknown".to_owned()
}
