package org.saga.audio

import android.content.Context
import android.content.SharedPreferences
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.saga.iroh.IrohNativeBridge

/**
 * §16 voice disguise preferences and live peer-chip state.
 *
 * Wording is always "voice disguise" (never "anonymous"). Does not affect
 * padlock / security state.
 */
object VoiceDisguiseStore {
    private const val PREFS = "saga_voice_disguise"
    private const val KEY_MODE = "mode"
    private const val KEY_PITCH = "pitch_semitones"
    private const val KEY_FORMANT = "formant_strength"
    private const val KEY_RANDOM = "random_each_call"
    private const val KEY_CALL_ENABLED = "call_enabled"
    private const val KEY_OVERRIDE_PREFIX = "override:"

    const val MODE_OFF = 0
    const val MODE_PITCH = 1
    const val MODE_FORMANT = 2
    const val MODE_FORMANT_PITCH = 3

    const val FORMANT_MILD = 0
    const val FORMANT_MEDIUM = 1
    const val FORMANT_STRONG = 2

    const val OVERRIDE_DEFAULT = 0
    const val OVERRIDE_ALWAYS_ON = 1
    const val OVERRIDE_ALWAYS_OFF = 2

    data class Settings(
        val mode: Int = MODE_OFF,
        val pitchSemitones: Float = -3f,
        val formantStrength: Int = FORMANT_MEDIUM,
        val randomEachCall: Boolean = true,
        /** In-call quick toggle (Iroh). When false, send path is dry. */
        val callEnabled: Boolean = true
    )

    private val peerBySession = ConcurrentHashMap<String, Boolean>()
    private val _peerDisguise = MutableStateFlow(false)
    val peerDisguise: StateFlow<Boolean> = _peerDisguise.asStateFlow()

    private val _localActive = MutableStateFlow(false)
    val localActive: StateFlow<Boolean> = _localActive.asStateFlow()

    private fun prefs(context: Context): SharedPreferences =
        context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun load(context: Context): Settings {
        val p = prefs(context)
        return Settings(
            mode = p.getInt(KEY_MODE, MODE_OFF).coerceIn(MODE_OFF, MODE_FORMANT_PITCH),
            pitchSemitones = p.getFloat(KEY_PITCH, -3f).coerceIn(-6f, 6f),
            formantStrength = p.getInt(KEY_FORMANT, FORMANT_MEDIUM)
                .coerceIn(FORMANT_MILD, FORMANT_STRONG),
            randomEachCall = p.getBoolean(KEY_RANDOM, true),
            callEnabled = p.getBoolean(KEY_CALL_ENABLED, true)
        )
    }

    fun save(context: Context, settings: Settings) {
        prefs(context).edit()
            .putInt(KEY_MODE, settings.mode.coerceIn(MODE_OFF, MODE_FORMANT_PITCH))
            .putFloat(KEY_PITCH, settings.pitchSemitones.coerceIn(-6f, 6f))
            .putInt(KEY_FORMANT, settings.formantStrength.coerceIn(FORMANT_MILD, FORMANT_STRONG))
            .putBoolean(KEY_RANDOM, settings.randomEachCall)
            .putBoolean(KEY_CALL_ENABLED, settings.callEnabled)
            .apply()
        pushNative(context, settings)
    }

    fun getContactOverride(context: Context, contactKey: String): Int {
        if (contactKey.isEmpty()) return OVERRIDE_DEFAULT
        return prefs(context)
            .getInt(KEY_OVERRIDE_PREFIX + contactKey, OVERRIDE_DEFAULT)
            .coerceIn(OVERRIDE_DEFAULT, OVERRIDE_ALWAYS_OFF)
    }

    fun setContactOverride(context: Context, contactKey: String, override: Int) {
        if (contactKey.isEmpty()) return
        prefs(context).edit()
            .putInt(
                KEY_OVERRIDE_PREFIX + contactKey,
                override.coerceIn(OVERRIDE_DEFAULT, OVERRIDE_ALWAYS_OFF)
            )
            .apply()
    }

    /**
     * Resolve effective settings for a call with an optional contact override key
     * (display name or peer id). Always-off forces mode Off; always-on forces a
     * non-off mode (pitch if global was Off).
     */
    fun effectiveForContact(context: Context, contactKey: String?): Settings {
        val base = load(context)
        return when (getContactOverride(context, contactKey.orEmpty())) {
            OVERRIDE_ALWAYS_OFF -> base.copy(mode = MODE_OFF, callEnabled = false)
            OVERRIDE_ALWAYS_ON -> {
                val mode = if (base.mode == MODE_OFF) MODE_PITCH else base.mode
                base.copy(mode = mode, callEnabled = true)
            }
            else -> base
        }
    }

    fun pushNative(context: Context, settings: Settings = load(context)) {
        IrohNativeBridge.setDisguiseSettings(
            settings.mode,
            settings.pitchSemitones,
            settings.formantStrength,
            settings.randomEachCall,
            settings.callEnabled && settings.mode != MODE_OFF
        )
        _localActive.value = settings.callEnabled && settings.mode != MODE_OFF
    }

    fun setCallEnabled(context: Context, enabled: Boolean) {
        val s = load(context).copy(callEnabled = enabled)
        save(context, s)
    }

    fun onPeerDisguise(sessionId: String, active: Boolean) {
        if (active) {
            peerBySession[sessionId] = true
        } else {
            peerBySession.remove(sessionId)
        }
        _peerDisguise.value = peerBySession.values.any { it }
    }

    fun clearPeer(sessionId: String) {
        peerBySession.remove(sessionId)
        _peerDisguise.value = peerBySession.values.any { it }
    }

    fun clearAllPeers() {
        peerBySession.clear()
        _peerDisguise.value = false
    }
}
