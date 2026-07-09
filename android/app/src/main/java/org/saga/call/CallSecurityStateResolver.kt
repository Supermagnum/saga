package org.saga.call

/**
 * Display states for the InCallService call screen (saga-spec.md section 13).
 * Priority: DOWNGRADED > KEY_NOT_FOUND > ENCRYPTION_POSSIBLE > ENCRYPTED.
 */
enum class PadlockDisplayState {
    SECURING,
    ENCRYPTED,
    KEY_NOT_FOUND,
    DOWNGRADED
}

data class CallSecuritySnapshot(
    val padlockState: PadlockDisplayState,
    val statusText: String,
    val playSecureCue: Boolean,
    val playUnsecureCue: Boolean,
    val showDowngradeModal: Boolean
)

/**
 * Resolves section 13 display state from call origin and handshake outcome.
 * Cellular calls always map to KEY_NOT_FOUND regardless of contact keys.
 */
object CallSecurityStateResolver {
    fun resolve(
        origin: CallOrigin,
        handshakeSecuring: Boolean = false,
        handshakeEncrypted: Boolean = false,
        handshakeFailed: Boolean = false,
        hasEncryptionHistory: Boolean = false,
        contactKeyResolvable: Boolean = false
    ): CallSecuritySnapshot {
        if (origin == CallOrigin.CELLULAR) {
            return CallSecuritySnapshot(
                padlockState = PadlockDisplayState.KEY_NOT_FOUND,
                statusText = "Not encrypted",
                playSecureCue = false,
                playUnsecureCue = true,
                showDowngradeModal = false
            )
        }

        if (handshakeFailed && hasEncryptionHistory) {
            return CallSecuritySnapshot(
                padlockState = PadlockDisplayState.DOWNGRADED,
                statusText = "Not encrypted",
                playSecureCue = false,
                playUnsecureCue = true,
                showDowngradeModal = true
            )
        }

        if (handshakeEncrypted) {
            return CallSecuritySnapshot(
                padlockState = PadlockDisplayState.ENCRYPTED,
                statusText = "Encrypted",
                playSecureCue = true,
                playUnsecureCue = false,
                showDowngradeModal = false
            )
        }

        if (handshakeSecuring && contactKeyResolvable) {
            return CallSecuritySnapshot(
                padlockState = PadlockDisplayState.SECURING,
                statusText = "Securing…",
                playSecureCue = false,
                playUnsecureCue = false,
                showDowngradeModal = false
            )
        }

        return CallSecuritySnapshot(
            padlockState = PadlockDisplayState.KEY_NOT_FOUND,
            statusText = "Not encrypted",
            playSecureCue = false,
            playUnsecureCue = true,
            showDowngradeModal = false
        )
    }

    fun displayPriority(state: PadlockDisplayState): Int = when (state) {
        PadlockDisplayState.DOWNGRADED -> 0
        PadlockDisplayState.KEY_NOT_FOUND -> 1
        PadlockDisplayState.SECURING -> 2
        PadlockDisplayState.ENCRYPTED -> 3
    }
}
