package org.saga.iroh

import android.net.Uri
import android.telephony.PhoneNumberUtils

object SagaCallUri {
    const val SCHEME = "saga"

    /**
     * Tel SSP for managed PhoneAccount placeCall / incoming address.
     *
     * Must produce **no ASCII digits 0-9** after
     * [PhoneNumberUtils.convertKeypadLettersToDigits] + [PhoneNumberUtils.stripSeparators],
     * and must still be accepted by Telecom for an enabled Saga PhoneAccount.
     *
     * Forensics (API 35 AVD, DEBUG_FORCE_SIM_PATH, account disabled):
     * - `"iroh"` → digits `4764` → modem `AT> ATD4764;` (unsafe)
     * - `"@"` → empty after strip → Telecom `ErrorDialogActivity` (blocks even enabled Saga dials)
     * - `"+"` → no digits, but modem `AT> AT+CUSD=1,"+"` (USSD — unsafe)
     * - `","` (PAUSE) → no digits; SIM path START_CONNECTION then abort; **no ATD/CUSD**
     *   (radio only saw hangup `AT+CLCC`). Enabled path reaches Saga ConnectionService.
     *
     * Avoid letters (keypad→digits), `*`/`#` (USSD), and `+` alone (CUSD).
     * Evaluated: managed `saga:` placeCall does not invoke ConnectionService on API 35;
     * `tel:` + EXTRA_PHONE_ACCOUNT_HANDLE does. Peer identity lives in EXTRA_PEER_ID only.
     */
    const val IROH_TELECOM_SSP = ","

    /** Former SSP — converts to digits `"4764"`; do not use. */
    const val LEGACY_UNSAFE_IROH_SSP = "iroh"

    fun forPeer(nodeId: IrohNodeId): Uri = Uri.fromParts(SCHEME, nodeId.raw, null)

    /** Telecom placeCall / incoming address — tel:, (PAUSE; no dialable digits). */
    fun telecomPlaceUri(@Suppress("UNUSED_PARAMETER") nodeId: IrohNodeId): Uri =
        Uri.fromParts("tel", IROH_TELECOM_SSP, null)

    /** Debug forensics only: build a tel URI with an arbitrary SSP. */
    fun telecomUriForSsp(ssp: String): Uri = Uri.fromParts("tel", ssp, null)

    /**
     * Characters remaining after keypad letter conversion + separator strip.
     * For safety we require **no ASCII digits 0-9** (and prefer fully empty).
     */
    fun afterKeypadAndStrip(ssp: String): String {
        val converted = PhoneNumberUtils.convertKeypadLettersToDigits(ssp)
        return PhoneNumberUtils.stripSeparators(converted).orEmpty()
    }

    fun dialableDigitsAfterNormalize(ssp: String): String =
        afterKeypadAndStrip(ssp).filter { it in '0'..'9' }

    fun isSafeTelecomSsp(ssp: String): Boolean = dialableDigitsAfterNormalize(ssp).isEmpty()

    fun parsePeer(uri: Uri?): IrohNodeId? {
        if (uri == null) return null
        val part = uri.schemeSpecificPart?.removePrefix("//")?.trim().orEmpty()
        if (part.isEmpty()) return null
        return when (uri.scheme?.lowercase()) {
            SCHEME -> IrohNodeId.parse(part) ?: IrohNodeId(part)
            "tel" -> {
                if (part == IROH_TELECOM_SSP ||
                    part.equals(LEGACY_UNSAFE_IROH_SSP, ignoreCase = true)
                ) {
                    return null
                }
                IrohNodeId.parse(part)
            }
            else -> null
        }
    }
}
