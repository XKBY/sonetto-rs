pub async fn get() -> String {
    default_check()
}

fn default_check() -> String {
    // Return empty resource lists for all regions.
    // This prevents the client from downloading any optional asset bundles
    // (audio packs, HD textures, config bundles) from official CDN servers.
    // Without this fix the handler would proxy to optionalres-hw.sl916.com when
    // online, fetching the current live resource manifest (4.5+ era) and causing
    // the client to cache and load official-version bundles that are incompatible
    // with this private server.
    String::from(
        r###"{"res-HD":{"res":[],"latest_ver":"101.65","download_url":"","download_url_bak":""},"jp":{"res":[],"latest_ver":"101.65","download_url":"","download_url_bak":""},"kr":{"res":[],"latest_ver":"101.65","download_url":"","download_url_bak":""},"zh":{"res":[],"latest_ver":"101.65","download_url":"","download_url_bak":""}}"###,
    )
}