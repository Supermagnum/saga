package org.saga.call

import org.junit.Assert.assertEquals
import org.junit.Test
import org.saga.R

class CallSecurityPresenterTest {
    @Test
    fun padlockDrawables_useVendoredIcons() {
        assertEquals(
            R.drawable.ic_saga_padlock_open,
            CallSecurityPresenter.padlockDrawableRes(
                CallSecuritySnapshot(PadlockDisplayState.SECURING, "Securing…", false, false, false)
            )
        )
        assertEquals(
            R.drawable.ic_saga_padlock_locked,
            CallSecurityPresenter.padlockDrawableRes(
                CallSecuritySnapshot(PadlockDisplayState.ENCRYPTED, "Encrypted", true, false, false)
            )
        )
        assertEquals(
            R.drawable.ic_saga_key_error,
            CallSecurityPresenter.padlockDrawableRes(
                CallSecuritySnapshot(PadlockDisplayState.KEY_NOT_FOUND, "Not encrypted", false, true, false)
            )
        )
        assertEquals(
            R.drawable.ic_saga_padlock_downgraded,
            CallSecurityPresenter.padlockDrawableRes(
                CallSecuritySnapshot(PadlockDisplayState.DOWNGRADED, "Not encrypted", false, true, true)
            )
        )
    }
}
