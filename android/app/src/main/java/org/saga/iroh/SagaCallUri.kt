package org.saga.iroh

import android.net.Uri

object SagaCallUri {
    const val SCHEME = "saga"

    fun forPeer(nodeId: IrohNodeId): Uri = Uri.fromParts(SCHEME, nodeId.raw, null)

    /** Telecom placeCall requires a tel: address; route via Saga PhoneAccount handle. */
    fun telecomPlaceUri(nodeId: IrohNodeId): Uri = Uri.fromParts("tel", nodeId.raw, null)

    fun parsePeer(uri: Uri?): IrohNodeId? {
        if (uri == null) return null
        val part = uri.schemeSpecificPart?.removePrefix("//")?.trim().orEmpty()
        if (part.isEmpty()) return null
        return when (uri.scheme?.lowercase()) {
            SCHEME, "tel" -> IrohNodeId.parse(part)
            else -> null
        }
    }
}
