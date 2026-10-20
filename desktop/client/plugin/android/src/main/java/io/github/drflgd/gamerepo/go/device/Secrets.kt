// The pairing token kept with a key the Android Keystore holds (D97): AES-256 in GCM, the key
// made once inside the Keystore and never leaving it; what it encrypts is kept in Game Repo
// Go's own preferences, unreadable to other apps and useless without the key.
package io.github.drflgd.gamerepo.go.device

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

object Secrets {
    private const val ALIAS = "Game Repo Go pairing"
    private const val PREFS = "gamerepo-go-secrets"

    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        gen.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build()
        )
        return gen.generateKey()
    }

    private fun prefs(ctx: Context) = ctx.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    @Synchronized
    fun get(ctx: Context, name: String): String? {
        val kept = prefs(ctx).getString(name, null) ?: return null
        val bytes = Base64.decode(kept, Base64.NO_WRAP)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, 12))
        return String(cipher.doFinal(bytes, 12, bytes.size - 12), Charsets.UTF_8)
    }

    @Synchronized
    fun set(ctx: Context, name: String, value: String) {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key())
        val sealed = cipher.iv + cipher.doFinal(value.toByteArray(Charsets.UTF_8))
        if (!prefs(ctx).edit().putString(name, Base64.encodeToString(sealed, Base64.NO_WRAP)).commit()) {
            throw IllegalStateException("Android didn't keep it")
        }
    }

    @Synchronized
    fun delete(ctx: Context, name: String) {
        prefs(ctx).edit().remove(name).commit()
    }
}
