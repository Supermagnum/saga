package org.saga.call

/**
 * Debug builds keep Iroh group calls for development.
 *
 * Release compiles this surface out (see src/release): G4/G5 failures mean a group
 * could show Encrypted without every participant's key verified — release must never
 * show that misleading padlock. Carrier conference (Phase 4) is unrelated and stays.
 */
object GroupCallFeatures {
    const val ENABLED: Boolean = true
}
