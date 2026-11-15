package org.saga.call

/**
 * Release: Iroh group calls are compiled out.
 *
 * Reason: G4/G5 failures mean a group could show Encrypted without every
 * participant's key verified. Release must never show that misleading padlock.
 * Carrier conference (Phase 4) is unchanged. Debug keeps the full mesh UI.
 */
object GroupCallFeatures {
    const val ENABLED: Boolean = false
}
