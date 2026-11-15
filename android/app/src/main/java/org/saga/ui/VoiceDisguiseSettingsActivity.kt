package org.saga.ui

import android.Manifest
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.os.Bundle
import android.view.View
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.saga.R
import org.saga.audio.SagaVoiceAudioEngine
import org.saga.audio.VoiceDisguiseStore
import org.saga.databinding.ActivityVoiceDisguiseSettingsBinding
import org.saga.iroh.IrohNativeBridge

/**
 * §16 Settings → Voice disguise. Wording never says "anonymous".
 * "Test my voice" records ~5 s locally, applies the current mode, plays back,
 * then deletes the recording (nothing leaves the phone).
 */
class VoiceDisguiseSettingsActivity : AppCompatActivity() {
    private lateinit var binding: ActivityVoiceDisguiseSettingsBinding
    private var suppressCallbacks = false

    private val requestMic = registerForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted ->
        if (granted) {
            runTestMyVoice()
        } else {
            Toast.makeText(this, R.string.voice_disguise_mic_denied, Toast.LENGTH_SHORT).show()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivityVoiceDisguiseSettingsBinding.inflate(layoutInflater)
        setContentView(binding.root)
        supportActionBar?.setDisplayHomeAsUpEnabled(true)
        title = getString(R.string.voice_disguise_title)

        suppressCallbacks = true
        bindFromStore(VoiceDisguiseStore.load(this))
        suppressCallbacks = false

        binding.modeGroup.setOnCheckedChangeListener { _, _ -> if (!suppressCallbacks) persist() }
        binding.formantGroup.setOnCheckedChangeListener { _, _ -> if (!suppressCallbacks) persist() }
        binding.pitchSlider.addOnChangeListener { _, _, fromUser ->
            if (fromUser && !suppressCallbacks) {
                updatePitchLabel()
                persist()
            }
        }
        binding.randomEachCall.setOnCheckedChangeListener { _, checked ->
            binding.fixedWarning.visibility = if (checked) View.GONE else View.VISIBLE
            if (!suppressCallbacks) persist()
        }
        binding.testMyVoice.setOnClickListener { onTestMyVoice() }
        binding.saveOverride.setOnClickListener { saveOverride() }
        updateSections()
    }

    override fun onSupportNavigateUp(): Boolean {
        finish()
        return true
    }

    private fun bindFromStore(s: VoiceDisguiseStore.Settings) {
        when (s.mode) {
            VoiceDisguiseStore.MODE_PITCH -> binding.modePitch.isChecked = true
            VoiceDisguiseStore.MODE_FORMANT -> binding.modeFormant.isChecked = true
            VoiceDisguiseStore.MODE_FORMANT_PITCH -> binding.modeFormantPitch.isChecked = true
            else -> binding.modeOff.isChecked = true
        }
        binding.pitchSlider.value = s.pitchSemitones
        when (s.formantStrength) {
            VoiceDisguiseStore.FORMANT_MILD -> binding.formantMild.isChecked = true
            VoiceDisguiseStore.FORMANT_STRONG -> binding.formantStrong.isChecked = true
            else -> binding.formantMedium.isChecked = true
        }
        binding.randomEachCall.isChecked = s.randomEachCall
        binding.fixedWarning.visibility = if (s.randomEachCall) View.GONE else View.VISIBLE
        binding.overrideDefault.isChecked = true
        updatePitchLabel()
        updateSections()
    }

    private fun currentSettings(): VoiceDisguiseStore.Settings {
        val mode = when (binding.modeGroup.checkedRadioButtonId) {
            R.id.mode_pitch -> VoiceDisguiseStore.MODE_PITCH
            R.id.mode_formant -> VoiceDisguiseStore.MODE_FORMANT
            R.id.mode_formant_pitch -> VoiceDisguiseStore.MODE_FORMANT_PITCH
            else -> VoiceDisguiseStore.MODE_OFF
        }
        val formant = when (binding.formantGroup.checkedRadioButtonId) {
            R.id.formant_mild -> VoiceDisguiseStore.FORMANT_MILD
            R.id.formant_strong -> VoiceDisguiseStore.FORMANT_STRONG
            else -> VoiceDisguiseStore.FORMANT_MEDIUM
        }
        return VoiceDisguiseStore.Settings(
            mode = mode,
            pitchSemitones = binding.pitchSlider.value,
            formantStrength = formant,
            randomEachCall = binding.randomEachCall.isChecked,
            callEnabled = VoiceDisguiseStore.load(this).callEnabled
        )
    }

    private fun persist() {
        updateSections()
        VoiceDisguiseStore.save(this, currentSettings())
    }

    private fun updateSections() {
        val mode = currentSettings().mode
        val usesPitch =
            mode == VoiceDisguiseStore.MODE_PITCH || mode == VoiceDisguiseStore.MODE_FORMANT_PITCH
        val usesFormant =
            mode == VoiceDisguiseStore.MODE_FORMANT || mode == VoiceDisguiseStore.MODE_FORMANT_PITCH
        binding.pitchSection.visibility = if (usesPitch) View.VISIBLE else View.GONE
        binding.formantSection.visibility = if (usesFormant) View.VISIBLE else View.GONE
    }

    private fun updatePitchLabel() {
        val v = binding.pitchSlider.value.toInt()
        binding.pitchLabel.text = getString(R.string.voice_disguise_pitch_fmt, v)
    }

    private fun saveOverride() {
        val name = binding.contactName.text?.toString()?.trim().orEmpty()
        if (name.isEmpty()) {
            Toast.makeText(this, R.string.voice_disguise_contact_name_required, Toast.LENGTH_SHORT)
                .show()
            return
        }
        val override = when (binding.overrideGroup.checkedRadioButtonId) {
            R.id.override_always_on -> VoiceDisguiseStore.OVERRIDE_ALWAYS_ON
            R.id.override_always_off -> VoiceDisguiseStore.OVERRIDE_ALWAYS_OFF
            else -> VoiceDisguiseStore.OVERRIDE_DEFAULT
        }
        VoiceDisguiseStore.setContactOverride(this, name, override)
        Toast.makeText(this, R.string.voice_disguise_override_saved, Toast.LENGTH_SHORT).show()
    }

    private fun onTestMyVoice() {
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO)
            != PackageManager.PERMISSION_GRANTED
        ) {
            requestMic.launch(Manifest.permission.RECORD_AUDIO)
            return
        }
        runTestMyVoice()
    }

    private fun runTestMyVoice() {
        persist()
        binding.testMyVoice.isEnabled = false
        binding.testStatus.setText(R.string.voice_disguise_test_recording)
        lifecycleScope.launch {
            val ok = withContext(Dispatchers.IO) { recordProcessPlayback() }
            binding.testMyVoice.isEnabled = true
            binding.testStatus.setText(
                if (ok) R.string.voice_disguise_test_done else R.string.voice_disguise_test_failed
            )
        }
    }

    /** ~5 s local record → disguise → playback; buffer discarded afterwards. */
    private fun recordProcessPlayback(): Boolean {
        val sr = SagaVoiceAudioEngine.SAMPLE_RATE
        val seconds = 5
        val totalSamples = sr * seconds
        val minBuf = AudioRecord.getMinBufferSize(
            sr,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT
        )
        if (minBuf <= 0) return false
        val record = AudioRecord(
            MediaRecorder.AudioSource.VOICE_COMMUNICATION,
            sr,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
            minBuf.coerceAtLeast(SagaVoiceAudioEngine.FRAME_BYTES * 4)
        )
        if (record.state != AudioRecord.STATE_INITIALIZED) {
            record.release()
            return false
        }
        val pcm = ShortArray(totalSamples)
        record.startRecording()
        var offset = 0
        while (offset < totalSamples) {
            val n = record.read(pcm, offset, totalSamples - offset)
            if (n <= 0) break
            offset += n
        }
        record.stop()
        record.release()

        val bytes = ByteBuffer.allocateDirect(offset * 2).order(ByteOrder.LITTLE_ENDIAN)
        for (i in 0 until offset) {
            bytes.putShort(pcm[i])
        }
        bytes.flip()
        IrohNativeBridge.processDisguisePcm(bytes)
        bytes.rewind()
        for (i in 0 until offset) {
            pcm[i] = bytes.short
        }

        val track = AudioTrack.Builder()
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build()
            )
            .setAudioFormat(
                AudioFormat.Builder()
                    .setSampleRate(sr)
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                    .build()
            )
            .setBufferSizeInBytes(offset * 2)
            .setTransferMode(AudioTrack.MODE_STATIC)
            .build()
        track.write(pcm, 0, offset)
        track.play()
        try {
            Thread.sleep((seconds * 1000L) + 200L)
        } catch (_: InterruptedException) {
        }
        track.stop()
        track.release()
        // Recording lives only in locals; nothing persisted.
        pcm.fill(0)
        return offset > sr
    }
}
