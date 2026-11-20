//! The account's skin: a PNG and its arm model sent to the Minecraft services
//! (`POST /minecraft/profile/skins`, a multipart form with `variant` and `file`), which every
//! client and server then shows, as the official launcher does.
use super::{bearer, request_to, str_field, Endpoints, Value};

/// The most a skin may weigh here: real ones are a few kilobytes.
pub const MAX_SKIN: usize = 256 * 1024;

/// Wears the skin `png` with slim or classic arms; the skin the profile wears after it (its URL
/// and model, "classic" or "slim").
pub fn upload(
    ep: &Endpoints,
    token: &str,
    png: &[u8],
    slim: bool,
) -> Result<Option<(String, String)>, String> {
    if png.len() > MAX_SKIN || crate::player_model::skin(png).is_none() {
        return Err("This isn't a Minecraft skin: it must be a 64×64 or 64×32 PNG.".into());
    }
    let mut boundary = [0u8; 12];
    getrandom::fill(&mut boundary).map_err(|e| e.to_string())?;
    let boundary: String = boundary.iter().map(|b| format!("{b:02x}")).collect();
    let boundary = format!("sunrise-{boundary}");
    let variant = if slim { "slim" } else { "classic" };
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"variant\"\r\n\r\n{variant}\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"skin.png\"\r\n\
         Content-Type: image/png\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(png);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let request = bearer(
        request_to("POST", &format!("{}/minecraft/profile/skins", ep.minecraft)),
        token,
    )
    .set(
        "Content-Type",
        &format!("multipart/form-data; boundary={boundary}"),
    );
    let response = match request.send_bytes(&body) {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(ureq::Error::Transport(_)) => {
            return Err(
                "Could not reach the Minecraft services. Check your internet connection.".into(),
            )
        }
    };
    let status = response.status();
    let profile: Value =
        serde_json::from_str(&response.into_string().unwrap_or_default()).unwrap_or(Value::Null);
    match status {
        200 => Ok(worn(&profile)),
        401 => Err(super::SIGN_IN_AGAIN.into()),
        400 => Err("Mojang refused this skin. It must be a 64×64 or 64×32 PNG.".into()),
        429 => Err("Too many skin changes in a short time. Wait a minute, then try again.".into()),
        _ => Err(format!(
            "The Minecraft services couldn't change the skin (error {status}). Try again later."
        )),
    }
}

/// The skin a profile wears: its URL and model.
pub fn worn(profile: &Value) -> Option<(String, String)> {
    let skin = profile
        .get("skins")?
        .as_array()?
        .iter()
        .find(|s| s.get("state").and_then(Value::as_str) == Some("ACTIVE"))?;
    Some((
        str_field(skin, "url")?,
        str_field(skin, "variant")
            .unwrap_or_else(|| "CLASSIC".into())
            .to_lowercase(),
    ))
}
