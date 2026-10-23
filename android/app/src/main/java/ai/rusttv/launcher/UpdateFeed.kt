package ai.rusttv.launcher

import java.net.URI
import org.json.JSONObject

/**
 * Parsing for [Updater]: the GitHub release feed, version tags, `SHA256SUMS`
 * and allowed download URLs (SECURITY.md § Launcher updates). No I/O and no
 * Android APIs except org.json, so it can be read (and later tested) alone.
 */
internal object UpdateFeed {
    /** The ABIs a release has an APK for, as named in the asset names (RELEASE.md). */
    private val ABIS = setOf("arm64-v8a", "armeabi-v7a", "x86_64")

    /**
     * Hosts a download may come from, redirects included: the API, release
     * pages, and the storage GitHub redirects release downloads to
     * (objects.githubusercontent.com, and release-assets. on newer setups).
     */
    private val HOSTS = setOf(
        "api.github.com",
        "github.com",
        "objects.githubusercontent.com",
        "release-assets.githubusercontent.com",
    )

    /** `v<major>.<minor>.<patch>`, no leading zeros; minor and patch below 100 like versionCode. */
    private val TAG = Regex("""^v(0|[1-9]\d{0,3})\.(0|[1-9]\d?)\.(0|[1-9]\d?)$""")

    data class Asset(val name: String, val url: String, val size: Long)

    data class Release(val version: String, val versionCode: Long, val assets: List<Asset>)

    /** The versionCode for a release tag (major * 10000 + minor * 100 + patch, as in build.gradle.kts), or null. */
    fun versionCode(tag: String): Long? {
        val (major, minor, patch) = TAG.find(tag)?.destructured ?: return null
        return major.toLong() * 10000 + minor.toLong() * 100 + patch.toLong()
    }

    /**
     * The release in GitHub's `releases/latest` answer, or null if it isn't
     * one the launcher may install: a draft, a pre-release, or a tag that
     * isn't a plain version. Throws org.json's JSONException on malformed JSON.
     */
    fun parseRelease(json: String): Release? {
        val root = JSONObject(json)
        if (root.optBoolean("draft", true) || root.optBoolean("prerelease", true)) return null
        val tag = root.getString("tag_name")
        val versionCode = versionCode(tag) ?: return null
        val list = root.getJSONArray("assets")
        val assets = (0 until list.length()).map { index ->
            val asset = list.getJSONObject(index)
            Asset(asset.getString("name"), asset.getString("browser_download_url"), asset.getLong("size"))
        }
        return Release(tag.removePrefix("v"), versionCode, assets)
    }

    /** The APK for the first of the device's ABIs (best first, `Build.SUPPORTED_ABIS`) the release has. */
    fun apkFor(release: Release, supportedAbis: Array<String>): Asset? =
        supportedAbis.asSequence()
            .filter { it in ABIS }
            .mapNotNull { abi -> release.assets.find { it.name == "rusttv-launcher-${release.version}-$abi.apk" } }
            .firstOrNull()

    /** The release's `SHA256SUMS` file, if it has one. */
    fun sumsFor(release: Release): Asset? = release.assets.find { it.name == "SHA256SUMS" }

    /**
     * The lower-case SHA-256 that a `shasum -a 256` listing gives for [name]
     * ("<hex>  <name>", or "<hex> *<name>" in binary mode). Null if the name
     * isn't listed, or is listed with two different sums.
     */
    fun sha256For(sums: String, name: String): String? {
        val found = sums.lineSequence()
            .map { it.trimEnd('\r') }
            .mapNotNull { line ->
                val hex = line.substringBefore(' ')
                val file = line.substringAfter(' ', "").trimStart(' ').removePrefix("*")
                if (file == name && hex.length == 64 && hex.all { it in "0123456789abcdefABCDEF" }) {
                    hex.lowercase()
                } else {
                    null
                }
            }
            .toSet()
        return found.singleOrNull()
    }

    /** Whether the launcher may fetch [url]: https on the default port, to a GitHub host above. */
    fun isAllowedUrl(url: String): Boolean {
        val uri = try {
            URI(url)
        } catch (e: java.net.URISyntaxException) {
            return false
        }
        return uri.scheme == "https" &&
            uri.rawUserInfo == null &&
            (uri.port == -1 || uri.port == 443) &&
            (uri.host ?: "").lowercase() in HOSTS
    }
}
