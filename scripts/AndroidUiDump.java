import android.app.UiAutomation;
import android.graphics.Rect;
import android.util.Xml;
import android.view.accessibility.AccessibilityNodeInfo;
import java.io.ByteArrayOutputStream;
import java.io.BufferedReader;
import java.io.FileOutputStream;
import java.io.InputStreamReader;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import org.xmlpull.v1.XmlSerializer;

/** Shell-only cloud test reader. Continuous progress updates need no idle wait. */
public final class AndroidUiDump {
    private static String text(CharSequence value) {
        return value == null ? "" : value.toString().replaceAll("[\\x00-\\x08\\x0B\\x0C\\x0E-\\x1F]", "");
    }

    private static void node(XmlSerializer xml, AccessibilityNodeInfo info, Rect viewport, boolean includeInvisible) throws Exception {
        xml.startTag("", "node");
        xml.attribute("", "text", text(info.getText()));
        xml.attribute("", "content-desc", text(info.getContentDescription()));
        xml.attribute("", "class", text(info.getClassName()));
        xml.attribute("", "package", text(info.getPackageName()));
        xml.attribute("", "enabled", Boolean.toString(info.isEnabled()));
        xml.attribute("", "visible", Boolean.toString(info.isVisibleToUser()));
        Rect bounds = new Rect();
        info.getBoundsInScreen(bounds);
        if (!bounds.intersect(viewport)) bounds.setEmpty();
        xml.attribute("", "bounds", "[" + bounds.left + "," + bounds.top + "][" + bounds.right + "," + bounds.bottom + "]");
        for (int index = 0; index < info.getChildCount(); index++) {
            AccessibilityNodeInfo child = info.getChild(index);
            if (child != null && (includeInvisible || child.isVisibleToUser())) node(xml, child, viewport, includeInvisible);
        }
        xml.endTag("", "node");
    }

    private static byte[] snapshot(UiAutomation automation, boolean includeInvisible) throws Exception {
        automation.clearCache();
        AccessibilityNodeInfo root = null;
        for (int attempt = 0; attempt < 20 && root == null; attempt++) {
            root = automation.getRootInActiveWindow();
            if (root == null) Thread.sleep(50);
        }
        if (root == null) throw new IllegalStateException("No active accessibility root");
        if (!root.refresh()) throw new IllegalStateException("Accessibility root changed");
        ByteArrayOutputStream output = new ByteArrayOutputStream();
        XmlSerializer xml = Xml.newSerializer();
        xml.setOutput(output, "UTF-8");
        xml.startDocument("UTF-8", true);
        xml.startTag("", "hierarchy");
        Rect viewport = new Rect();
        root.getBoundsInScreen(viewport);
        node(xml, root, viewport, includeInvisible);
        xml.endTag("", "hierarchy");
        xml.endDocument();
        return output.toByteArray();
    }

    private static void run(String[] args) throws Exception {
        Class<?> type = Class.forName("com.android.uiautomator.core.UiAutomationShellWrapper");
        Object wrapper = type.getDeclaredConstructor().newInstance();
        type.getMethod("connect").invoke(wrapper);
        try {
            type.getMethod("setCompressedLayoutHierarchy", boolean.class).invoke(wrapper, true);
            UiAutomation automation = (UiAutomation) type.getMethod("getUiAutomation").invoke(wrapper);
            if ("--serve".equals(args[0])) {
                try (ServerSocket server = new ServerSocket(17891, 4, InetAddress.getByName("127.0.0.1"))) {
                    boolean running = true;
                    while (running) {
                        try (Socket client = server.accept()) {
                            client.setSoTimeout(10000);
                            String command = new BufferedReader(new InputStreamReader(client.getInputStream(), "UTF-8")).readLine();
                            byte[] response;
                            if ("STOP".equals(command)) {
                                response = "BYE".getBytes("UTF-8");
                                running = false;
                            } else if ("PING".equals(command)) {
                                response = "READY".getBytes("UTF-8");
                            } else if ("DUMP".equals(command) || "DUMP_ALL".equals(command)) {
                                try {
                                    response = snapshot(automation, "DUMP_ALL".equals(command));
                                } catch (Exception error) {
                                    response = ("ERROR " + error.getClass().getSimpleName()).getBytes("UTF-8");
                                }
                            } else {
                                response = "ERROR unknown command".getBytes("UTF-8");
                            }
                            client.getOutputStream().write((response.length + "\n").getBytes("UTF-8"));
                            client.getOutputStream().write(response);
                            client.getOutputStream().flush();
                        }
                    }
                }
            } else {
                try (FileOutputStream output = new FileOutputStream(args[0])) {
                    output.write(snapshot(automation, false));
                }
                System.out.println("UI hierarchy dumped to: " + args[0]);
            }
        } finally {
            type.getMethod("disconnect").invoke(wrapper);
        }
    }

    public static void main(String[] args) {
        try {
            run(args);
        } catch (Exception error) {
            error.printStackTrace(System.err);
            System.out.println("UI dump failed: " + error.getClass().getSimpleName());
            System.exit(1);
        }
    }
}
