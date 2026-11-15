package org.saga.dial

import android.content.Context

/**
 * Release stub: Add-participant picker is debug-only.
 */
object IrohContactDirectory {
    data class IrohContact(val displayName: String, val target: DialTarget.Iroh)

    fun list(context: Context, excludeLookupKeys: Set<String> = emptySet()): List<IrohContact> =
        emptyList()
}
