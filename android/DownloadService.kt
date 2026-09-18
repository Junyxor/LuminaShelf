package app.luminashelf.client

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import app.tauri.plugin.Channel
import app.tauri.plugin.JSObject
import java.util.concurrent.ConcurrentHashMap

class DownloadService : Service() {
    private var wakeLock: PowerManager.WakeLock? = null
    private data class Transfer(val title: String, val progress: Int, val control: Channel)

    companion object {
        const val UPDATE = "app.luminashelf.client.DOWNLOAD_UPDATE"
        const val PAUSE_ALL = "app.luminashelf.client.DOWNLOAD_PAUSE"
        private const val CHANNEL = "luminashelf_downloads"
        private const val NOTIFICATION = 6101
        private val tasks = ConcurrentHashMap<String, Transfer>()
        @Volatile private var service: DownloadService? = null

        fun register(id: String, title: String, control: Channel) {
            tasks[id] = Transfer(title, -1, control)
        }
        fun remove(id: String) { tasks.remove(id) }
        fun update(id: String, title: String, progress: Int, active: Boolean) {
            if (active) tasks.computeIfPresent(id) { _, previous -> previous.copy(title = title, progress = progress) }
            else tasks.remove(id)
            service?.refresh()
        }
        private fun pauseAll(reason: String) {
            tasks.forEach { (id, task) ->
                task.control.send(JSObject().apply { put("id", id); put("action", "pause"); put("reason", reason) })
            }
        }
    }

    override fun onCreate() {
        super.onCreate()
        service = this
        if (Build.VERSION.SDK_INT >= 26) {
            getSystemService(NotificationManager::class.java).createNotificationChannel(
                NotificationChannel(CHANNEL, "电子书下载", NotificationManager.IMPORTANCE_LOW).apply {
                    description = "后台下载进度与暂停操作"
                    setShowBadge(false)
                }
            )
        }
        wakeLock = (getSystemService(POWER_SERVICE) as PowerManager).newWakeLock(
            PowerManager.PARTIAL_WAKE_LOCK, "LuminaShelf:Downloads"
        ).apply { setReferenceCounted(false) }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == PAUSE_ALL) {
            pauseAll("notification")
            return START_NOT_STICKY
        }
        // Enter foreground immediately, even if a fast transfer already finished.
        ServiceCompat.startForeground(this, NOTIFICATION, notification(),
            if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC else 0)
        if (tasks.isEmpty()) stopFinished() else {
            if (wakeLock?.isHeld != true) wakeLock?.acquire(6 * 60 * 60 * 1000L)
        }
        return START_NOT_STICKY
    }

    private fun notification(): Notification {
        val open = PendingIntent.getActivity(this, 0,
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val pause = PendingIntent.getService(this, 1,
            Intent(this, DownloadService::class.java).setAction(PAUSE_ALL),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val snapshot = tasks.values.toList()
        val first = snapshot.firstOrNull()
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(if (snapshot.size > 1) "正在下载 ${snapshot.size} 本书" else first?.title ?: "下载已完成")
            .setContentText(if (first != null && first.progress >= 0) "${first.progress}% · 可锁屏或切换应用" else "正在连接书源…")
            .setProgress(100, first?.progress?.coerceAtLeast(0) ?: 0, first?.progress == null || first.progress < 0)
            .setContentIntent(open)
            .addAction(android.R.drawable.ic_media_pause, "全部暂停", pause)
            .setOngoing(true).setOnlyAlertOnce(true).setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_PROGRESS).build()
    }

    fun refresh() {
        android.os.Handler(mainLooper).post {
            if (tasks.isEmpty()) stopFinished()
            else getSystemService(NotificationManager::class.java).notify(NOTIFICATION, notification())
        }
    }

    private fun stopFinished() {
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        if (wakeLock?.isHeld == true) wakeLock?.release()
        stopSelf()
    }

    // Android 15 limits dataSync foreground services to six hours in a 24h window.
    override fun onTimeout(startId: Int, fgsType: Int) {
        pauseAll("system_timeout")
        stopFinished()
    }

    override fun onDestroy() {
        if (tasks.isNotEmpty()) pauseAll("service_destroyed")
        service = null
        if (wakeLock?.isHeld == true) wakeLock?.release()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null
}
