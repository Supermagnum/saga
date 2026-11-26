package org.saga.contacts

import org.saga.iroh.ContactKeyStore
import java.net.URLDecoder
import java.net.URLEncoder
import java.nio.charset.StandardCharsets

/**
 * Versioned add-contact link for QR / paste / NFC / import.
 *
 * Form: `saga://add-contact?v=1&fp=<64hex>[&ep=<endpoint>][&name=<urlenc>]`
 *
 * Distinct from call URIs (`saga:<nodeId>` / [org.saga.iroh.SagaCallUri]).
 * Pure string parse/build (no android.net.Uri) so JVM unit tests can round-trip.
 */
object SagaAddContactLink {
    const val SCHEME = "saga"
    const val HOST = "add-contact"
    const val VERSION = 1

    data class Payload(
        val fingerprintHex: String,
        val endpointId: String? = null,
        val suggestedName: String? = null
    )

    sealed class ParseError {
        data object WrongScheme : ParseError()
        data object WrongHost : ParseError()
        data object UnknownVersion : ParseError()
        data object MissingFingerprint : ParseError()
        data object BadFingerprint : ParseError()
        data object BadEndpoint : ParseError()
        data object DuplicateParam : ParseError()
        data object ExtraParam : ParseError()
        data object Malformed : ParseError()
    }

    fun build(payload: Payload): String {
        val fp = payload.fingerprintHex.trim().lowercase()
        require(ContactKeyStore.isFingerprintHex(fp)) { "fingerprint must be 64 lowercase hex" }
        val parts = mutableListOf(
            "v=$VERSION",
            "fp=$fp"
        )
        val ep = payload.endpointId?.trim()?.takeIf { it.isNotEmpty() }
        if (ep != null) {
            require(isPlausibleEndpointId(ep)) { "endpoint id looks invalid" }
            parts.add("ep=${urlEncode(ep)}")
        }
        val name = payload.suggestedName?.trim()?.takeIf { it.isNotEmpty() }
        if (name != null) {
            parts.add("name=${urlEncode(name)}")
        }
        return "$SCHEME://$HOST?${parts.joinToString("&")}"
    }

    fun parse(raw: String): Result<Payload> {
        val trimmed = raw.trim()
        val schemeSep = trimmed.indexOf("://")
        if (schemeSep <= 0) {
            return fail(ParseError.WrongScheme)
        }
        val scheme = trimmed.substring(0, schemeSep)
        if (!SCHEME.equals(scheme, ignoreCase = true)) {
            return fail(ParseError.WrongScheme)
        }
        val afterScheme = trimmed.substring(schemeSep + 3)
        val querySep = afterScheme.indexOf('?')
        if (querySep < 0) {
            return fail(ParseError.Malformed)
        }
        val host = afterScheme.substring(0, querySep)
        if (!HOST.equals(host, ignoreCase = true)) {
            return fail(ParseError.WrongHost)
        }
        val query = afterScheme.substring(querySep + 1)
        val params = LinkedHashMap<String, String>()
        if (query.isNotEmpty()) {
            for (pair in query.split('&')) {
                if (pair.isEmpty()) continue
                val eq = pair.indexOf('=')
                val key = if (eq >= 0) pair.substring(0, eq) else pair
                val value = if (eq >= 0) pair.substring(eq + 1) else ""
                val decodedKey = urlDecode(key)
                if (decodedKey in params) {
                    return fail(ParseError.DuplicateParam)
                }
                params[decodedKey] = urlDecode(value)
            }
        }
        val allowed = setOf("v", "fp", "ep", "name")
        for (n in params.keys) {
            if (n !in allowed) {
                return fail(ParseError.ExtraParam)
            }
        }
        val v = params["v"]?.toIntOrNull()
        if (v != VERSION) {
            return fail(ParseError.UnknownVersion)
        }
        val fpRaw = params["fp"]?.trim()?.lowercase()
            ?: return fail(ParseError.MissingFingerprint)
        if (!ContactKeyStore.isFingerprintHex(fpRaw)) {
            return fail(ParseError.BadFingerprint)
        }
        val ep = params["ep"]?.trim()?.takeIf { it.isNotEmpty() }
        if (ep != null && !isPlausibleEndpointId(ep)) {
            return fail(ParseError.BadEndpoint)
        }
        val name = params["name"]?.trim()?.takeIf { it.isNotEmpty() }
        return Result.success(Payload(fpRaw, ep, name))
    }

    /**
     * Iroh EndpointId as hex (64) or z-base-32 / base32 (typically 52 chars), or
     * debug phone-digit peer labels (8+ digits).
     */
    fun isPlausibleEndpointId(raw: String): Boolean {
        val s = raw.trim()
        if (s.length < 8 || s.length > 128) return false
        if (s.all { it.isDigit() } && s.length >= 8) return true
        if (Regex("^[0-9a-fA-F]{64}$").matches(s)) return true
        // z-base-32 / base32 alphabet used by iroh string forms
        if (Regex("^[a-z2-7A-Z]+=*$").matches(s) && s.length in 50..60) return true
        return false
    }

    private fun fail(error: ParseError): Result<Payload> =
        Result.failure(IllegalArgumentException(error.toString()))

    private fun urlEncode(value: String): String =
        URLEncoder.encode(value, StandardCharsets.UTF_8).replace("+", "%20")

    private fun urlDecode(value: String): String =
        URLDecoder.decode(value.replace("+", "%20"), StandardCharsets.UTF_8)
}
