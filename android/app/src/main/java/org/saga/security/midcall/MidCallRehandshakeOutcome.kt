package org.saga.security.midcall

enum class MidCallRehandshakeOutcome {
    SUCCESS,
    CRYPTO_FAILURE,
    TIMEOUT;

    companion object {
        fun fromIntentExtras(succeed: Boolean, failureMode: String?): MidCallRehandshakeOutcome {
            if (succeed) return SUCCESS
            return when (failureMode?.lowercase()) {
                "timeout" -> TIMEOUT
                else -> CRYPTO_FAILURE
            }
        }
    }
}
