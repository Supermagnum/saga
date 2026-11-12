package org.saga.iroh

import android.net.Uri

object SagaCallUri {
    const val SCHEME = "saga"

    /**
     * Non-routable tel SSP for managed PhoneAccount placeCall / incoming address.
     * Not E.164 — carriers cannot dial this. Peer identity lives in EXTRA_PEER_ID.
     *
     * Evaluated on API 35 AVD: managed `saga:` placeCall is accepted without throw but
     * does not invoke ConnectionService; `tel:` + EXTRA_PHONE_ACCOUNT_HANDLE does.
     */
    const val IROH_TELECOM_SSP = "iroh"

    fun forPeer(nodeId: IrohNodeId): Uri = Uri.fromParts(SCHEME, nodeId.raw, null)

    /** Telecom placeCall / incoming address — tel:iroh (never a routable E.164). */
    fun telecomPlaceUri(@Suppress("UNUSED_PARAMETER") nodeId: IrohNodeId): Uri =
        Uri.fromParts("tel", IROH_TELECOM_SSP, null)

    fun parsePeer(uri: Uri?): IrohNodeId? {
        if (uri == null) return null
        val part = uri.schemeSpecificPart?.removePrefix("//")?.trim().orEmpty()
        if (part.isEmpty()) return null
        return when (uri.scheme?.lowercase()) {
            SCHEME -> IrohNodeId.parse(part) ?: IrohNodeId(part)
            "tel" -> {
                if (part.equals(IROH_TELECOM_SSP, ignoreCase = true)) return null
                IrohNodeId.parse(part)
            }
            else -> null
        }
    }
}
