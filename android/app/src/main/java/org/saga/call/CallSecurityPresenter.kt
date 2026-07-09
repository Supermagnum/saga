package org.saga.call

import android.content.Context
import org.saga.security.connect.SagaConnectSecurityCuePlayer

/**
 * Applies connect-time security snapshot: audio cues and UI binding data.
 */
object CallSecurityPresenter {
    fun applyConnectSnapshot(context: Context, snapshot: CallSecuritySnapshot) {
        when {
            snapshot.playSecureCue -> SagaConnectSecurityCuePlayer.playSecure(context)
            snapshot.playUnsecureCue -> SagaConnectSecurityCuePlayer.playUnsecure(context)
        }
    }

    fun padlockContentDescription(snapshot: CallSecuritySnapshot): Int = when (snapshot.padlockState) {
        PadlockDisplayState.SECURING -> org.saga.R.string.padlock_securing
        PadlockDisplayState.ENCRYPTED -> org.saga.R.string.padlock_encrypted
        PadlockDisplayState.KEY_NOT_FOUND -> org.saga.R.string.padlock_not_encrypted
        PadlockDisplayState.DOWNGRADED -> org.saga.R.string.padlock_downgraded
    }

    fun padlockDrawableRes(snapshot: CallSecuritySnapshot): Int = when (snapshot.padlockState) {
        PadlockDisplayState.SECURING -> org.saga.R.drawable.ic_saga_padlock_open
        PadlockDisplayState.ENCRYPTED -> org.saga.R.drawable.ic_saga_padlock_locked
        PadlockDisplayState.KEY_NOT_FOUND -> org.saga.R.drawable.ic_saga_key_error
        PadlockDisplayState.DOWNGRADED -> org.saga.R.drawable.ic_saga_padlock_downgraded
    }
}
