import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.security.MessageDigest;

/** Independent Android platform-TLS control; only requests a public-domain book. */
public final class AndroidNetworkProbe {
    public static void main(String[] args) throws Exception {
        long started = System.nanoTime();
        long bytes = 0;
        HttpURLConnection connection = (HttpURLConnection) new URL(args[0]).openConnection();
        connection.setConnectTimeout(25000);
        connection.setReadTimeout(25000);
        connection.setUseCaches(false);
        connection.setRequestProperty("Accept-Encoding", "identity");
        connection.setRequestProperty("User-Agent", "LuminaShelf file transfer");
        if (args.length > 1 && "probe".equals(args[1])) connection.setRequestProperty("Range", "bytes=0-0");
        MessageDigest digest = MessageDigest.getInstance("SHA-256");
        try {
            System.out.println("status=" + connection.getResponseCode());
            System.out.println("length=" + connection.getContentLengthLong());
            System.out.println("contentRange=" + connection.getHeaderField("Content-Range"));
            try (InputStream input = connection.getInputStream()) {
                byte[] buffer = new byte[8192];
                int count;
                while ((count = input.read(buffer)) != -1) {
                    bytes += count;
                    digest.update(buffer, 0, count);
                }
            }
            StringBuilder hash = new StringBuilder();
            for (byte value : digest.digest()) hash.append(String.format("%02x", value & 255));
            System.out.println("sha256=" + hash);
        } finally {
            System.out.println("bytes=" + bytes);
            System.out.println("elapsedMs=" + (System.nanoTime() - started) / 1000000);
            connection.disconnect();
        }
    }
}
