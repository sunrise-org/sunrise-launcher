use super::*;

const REPO: &str = "owner/project";
const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn markdown_strips_html_and_only_opens_safe_https_links_from_the_pinned_directory() {
    let readme = Saved {
        path: "docs/README.md".into(),
        markdown: "# Atlas\n\n<i>Native text</i> and [`next`](guide.md#install).\n\n<script>nope()</script>\n".into(),
    }.parse(REPO, SHA, true);
    assert!(readme.markdown().contains("Native text"));
    assert!(!readme.markdown().contains("<i>"));
    assert!(!readme.markdown().contains("nope()"));
    assert_eq!(
        readme.link("guide.md#install"),
        Some(format!(
            "https://github.com/{REPO}/blob/{SHA}/docs/guide.md#install"
        ))
    );
    assert_eq!(
        readme.link("../LICENSE"),
        Some(format!("https://github.com/{REPO}/blob/{SHA}/LICENSE"))
    );
    assert_eq!(
        readme.link("https://example.test/"),
        Some("https://example.test/".into())
    );
    for refused in [
        "javascript:alert(1)",
        "http://example.test/",
        "https://user@example.test/a",
        "https://example.test:444/",
        "//example.test/a",
        "..\\LICENSE",
    ] {
        assert!(readme.link(refused).is_none(), "{refused}");
    }
}

#[test]
fn only_eight_same_repository_pinned_images_can_be_loaded() {
    let mut markdown = [
        "![not here](https://example.test/track.png)",
        "![not here](//raw.githubusercontent.com/owner/project/main/camps.png)",
        "![not here](https://raw.githubusercontent.com/owner/project/main/camps.png)",
        "![not here](../camps.png)",
        "![not here](media/%2e%2e/camps.png)",
        "![not here](https://user@raw.githubusercontent.com/owner/project/main/camps.png)",
        "![First **camp**](media/first.png)",
    ]
    .join("\n\n");
    markdown.push_str(&format!(
        "\n\n![second](https://raw.githubusercontent.com/{REPO}/{SHA}/other.jpg)"
    ));
    for n in 0..10 {
        markdown.push_str(&format!("\n\n![more](/media/{n}.webp)"));
    }
    let readme = Saved {
        path: "docs/README.md".into(),
        markdown,
    }
    .parse(REPO, SHA, false);
    let images = readme.images();
    assert_eq!(images.len(), 8);
    assert_eq!(images[0].alt(), "First camp");
    assert_eq!(images[0].destination(), "media/first.png");
    assert_eq!(images[0].path, "docs/media/first.png");
    assert_eq!(images[1].path, "other.jpg");
    assert_eq!(images[7].path, "media/5.webp");
    assert!(images.iter().all(|image| !image.allow_fetch));
}

#[test]
fn bytes_not_file_extensions_determine_if_a_bounded_still_image_is_safe() {
    let png = |width| {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(width, 1)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    };
    assert_eq!(check_image(&png(2048)).unwrap(), "png");
    assert!(check_image(&png(2049)).is_err());
    let mut too_big = png(16);
    too_big.resize(MAX_IMAGE as usize + 1, 0);
    assert!(check_image(&too_big).is_err());
    assert!(check_image(b"GIF89a").is_err());
    assert!(check_image(b"<svg></svg>").is_err());
    assert!(markdown_file("docs/README.MD"));
    assert!(!markdown_file("README.html"));
}
