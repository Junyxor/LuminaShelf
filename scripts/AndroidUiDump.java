import android.app.UiAutomation;
import android.graphics.Rect;
import android.util.Xml;
import android.view.accessibility.AccessibilityNodeInfo;
import java.io.FileOutputStream;
import org.xmlpull.v1.XmlSerializer;

/** Shell-only cloud test reader. Continuous progress updates need no idle wait. */
public final class AndroidUiDump {
    private static String text(CharSequence value) {
        return value == null ? "" : value.toString().replaceAll("[\\x00-\\x08\\x0B\\x0C\\x0E-\\x1F]", "");
    }

    private static void node(XmlSerializer xml, AccessibilityNodeInfo info, Rect viewport) throws Exception {
        xml.startTag("", "node");
        xml.attribute("", "text", text(info.getText()));
        xml.attribute("", "content-desc", text(info.getContentDescription()));
        xml.attribute("", "class", text(info.getClassName()));
        xml.attribute("", "package", text(info.getPackageName()));
        xml.attribute("", "enabled", Boolean.toString(info.isEnabled()));
        Rect bounds = new Rect();
        info.getBoundsInScreen(bounds);
        if (!bounds.intersect(viewport)) bounds.setEmpty();
        xml.attribute("", "bounds", "[" + bounds.left + "," + bounds.top + "][" + bounds.right + "," + bounds.bottom + "]");
        for (int index = 0; index < info.getChildCount(); index++) {
            AccessibilityNodeInfo child = info.getChild(index);
            if (child != null && child.isVisibleToUser()) node(xml, child, viewport);
        }
        xml.endTag("", "node");
    }

    private static void dump(String destination) throws Exception {
        Class<?> type = Class.forName("com.android.uiautomator.core.UiAutomationShellWrapper");
        Object wrapper = type.getDeclaredConstructor().newInstance();
        type.getMethod("connect").invoke(wrapper);
        try {
            type.getMethod("setCompressedLayoutHierarchy", boolean.class).invoke(wrapper, true);
            UiAutomation automation = (UiAutomation) type.getMethod("getUiAutomation").invoke(wrapper);
            AccessibilityNodeInfo root = null;
            for (int attempt = 0; attempt < 20 && root == null; attempt++) {
                root = automation.getRootInActiveWindow();
                if (root == null) Thread.sleep(50);
            }
            if (root == null) throw new IllegalStateException("No active accessibility root");
            try (FileOutputStream output = new FileOutputStream(destination)) {
                XmlSerializer xml = Xml.newSerializer();
                xml.setOutput(output, "UTF-8");
                xml.startDocument("UTF-8", true);
                xml.startTag("", "hierarchy");
                Rect viewport = new Rect();
                root.getBoundsInScreen(viewport);
                node(xml, root, viewport);
                xml.endTag("", "hierarchy");
                xml.endDocument();
            }
            System.out.println("UI hierarchy dumped to: " + destination);
        } finally {
            type.getMethod("disconnect").invoke(wrapper);
        }
    }

    public static void main(String[] args) {
        try {
            dump(args[0]);
        } catch (Exception error) {
            error.printStackTrace(System.err);
            System.out.println("UI dump failed: " + error.getClass().getSimpleName());
            System.exit(1);
        }
    }
}
