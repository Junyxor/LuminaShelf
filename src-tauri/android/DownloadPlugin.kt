package app.luminashelf.client

import android.app.Activity
import android.content.Intent
import androidx.annotation.Keep
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

@TauriPlugin
class DownloadPlugin(private val activity: Activity) : Plugin(activity) {
    @Command
    fun setActive(invoke: Invoke) {
        val context = activity.applicationContext
        try {
            if (invoke.getArgs().optBoolean("active", false)) {
                val intent = Intent(context, DownloadService::class.java)
                if (DownloadService.running) context.startService(intent)
                else ContextCompat.startForegroundService(context, intent)
            } else {
                context.stopService(Intent(context, DownloadService::class.java))
            }
            invoke.resolve()
        } catch (error: Exception) {
            android.util.Log.w("LuminaDownloads", "Could not update download foreground service", error)
            invoke.reject("后台保活未能启动")
        }
    }
}

@Keep
object DownloadNative {
    init { System.loadLibrary("lumina_shelf_desktop") }
    @JvmStatic external fun pauseAll()
}
