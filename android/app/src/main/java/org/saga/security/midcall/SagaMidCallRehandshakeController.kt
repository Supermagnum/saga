package org.saga.security.midcall

import android.content.Context
import org.saga.iroh.IrohDialManager

/**
 * Debug/test entry for section 5b mid-call re-handshake. Routes through [IrohDialManager]
 * so downgrade logging, registry updates, and modal state match Case 4.
 */
object SagaMidCallRehandshakeController {
    fun simulateNetworkPathChange(
        context: Context,
        telecomCallId: String?,
        lookupKey: String,
        succeed: Boolean
    ) {
        val outcome = if (succeed) MidCallRehandshakeOutcome.SUCCESS else MidCallRehandshakeOutcome.CRYPTO_FAILURE
        trigger(context, telecomCallId, lookupKey, outcome)
    }

    fun trigger(
        context: Context,
        telecomCallId: String?,
        lookupKey: String,
        outcome: MidCallRehandshakeOutcome
    ) {
        IrohDialManager.get(context).handleMidCallRehandshake(telecomCallId, lookupKey, outcome)
    }
}
