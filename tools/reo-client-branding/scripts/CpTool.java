import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.zip.*;

/**
 * Constant-pool string tool for .class files inside jars.
 *
 *   --scan  <jar> [values...]           list Utf8 entries exactly equal to any value
 *   --apply <in.jar> <out.jar> <pairs>  copy jar, replacing exact Utf8 constants
 *                                       (pairs file lines: old=>new, UTF-8)
 *
 * Replacement is done only in the constant pool (tag 1 entries). Indexes,
 * bytecode offsets and stack map frames are untouched, so classes stay valid
 * even when the new string differs in length.
 */
public final class CpTool {
    static final class Utf8 {
        final int start; // offset of tag byte
        final int end;   // offset just past payload
        final String value;
        Utf8(int start, int end, String value) { this.start = start; this.end = end; this.value = value; }
    }

    static List<Utf8> parse(byte[] b) throws IOException {
        DataInputStream in = new DataInputStream(new ByteArrayInputStream(b));
        in.readInt();
        in.readUnsignedShort();
        in.readUnsignedShort();
        int cpCount = in.readUnsignedShort();
        int offset = 10;
        List<Utf8> out = new ArrayList<>();
        for (int i = 1; i < cpCount; i++) {
            int tag = in.readUnsignedByte();
            int start = offset;
            offset += 1;
            switch (tag) {
                case 1: {
                    in.mark(2);
                    int len = in.readUnsignedShort();
                    in.reset();
                    String s = in.readUTF();
                    offset += 2 + len;
                    out.add(new Utf8(start, offset, s));
                    break;
                }
                case 3: case 4: skip(in, 4); offset += 4; break;
                case 5: case 6: skip(in, 8); offset += 8; i++; break;
                case 7: case 8: case 16: case 19: case 20: skip(in, 2); offset += 2; break;
                case 9: case 10: case 11: case 12: case 17: case 18: skip(in, 4); offset += 4; break;
                case 15: skip(in, 3); offset += 3; break;
                default: throw new IOException("unknown constant pool tag " + tag + " at index " + i);
            }
        }
        return out;
    }

    static void skip(DataInputStream in, int n) throws IOException {
        int left = n;
        while (left > 0) {
            int r = in.skipBytes(left);
            if (r <= 0) throw new EOFException();
            left -= r;
        }
    }

    static byte[] replace(byte[] b, Map<String, String> pairs) throws IOException {
        List<Utf8> entries = parse(b);
        ByteArrayOutputStream out = new ByteArrayOutputStream(b.length + 64);
        int cursor = 0;
        boolean any = false;
        for (Utf8 e : entries) {
            String to = pairs.get(e.value);
            if (to == null) continue;
            out.write(b, cursor, e.start - cursor);
            out.write(1);
            ByteArrayOutputStream tmp = new ByteArrayOutputStream();
            DataOutputStream dos = new DataOutputStream(tmp);
            dos.writeUTF(to);
            dos.flush();
            out.write(tmp.toByteArray());
            cursor = e.end;
            any = true;
        }
        if (!any) return null;
        out.write(b, cursor, b.length - cursor);
        return out.toByteArray();
    }

    /**
     * Display-string policy used by the ReoClient wordmark pass:
     *   ONECLIENT            -> REOCLIENT
     *   prose "OneClient..." -> "ReoClient..." (identifiers, URLs, package names skipped)
     */
    static String brand(String value) {
        if (value.equals("ONECLIENT")) return "REOCLIENT";
        if (!value.contains("OneClient")) return null;
        if (value.contains("://") || value.startsWith("http")) return null;
        boolean prose = value.equals("OneClient") || value.matches("(?s).*[\\s<>!?,;:'\"].*");
        if (!prose) return null;
        if (value.matches("^[A-Za-z0-9_$]+$")) return null;
        return value.replace("OneClient", "ReoClient");
    }

    static byte[] replaceBranding(byte[] b) throws IOException {
        List<Utf8> entries = parse(b);
        Map<String, String> pairs = new LinkedHashMap<>();
        for (Utf8 e : entries) {
            String to = brand(e.value);
            if (to != null && !to.equals(e.value)) pairs.put(e.value, to);
        }
        if (pairs.isEmpty()) return null;
        return replace(b, pairs);
    }

    static Map<String, String> readPairs(String path) throws IOException {
        Map<String, String> map = new LinkedHashMap<>();
        for (String line : Files.readAllLines(Paths.get(path), StandardCharsets.UTF_8)) {
            if (line.isEmpty() || line.startsWith("#")) continue;
            int i = line.indexOf("=>");
            if (i < 0) throw new IOException("bad pair line: " + line);
            map.put(line.substring(0, i), line.substring(i + 2));
        }
        return map;
    }

    public static void main(String[] args) throws Exception {
        if (args.length == 3 && args[0].equals("--normalize")) {
            try (ZipFile zf = new ZipFile(args[1]);
                 ZipOutputStream zout = new ZipOutputStream(new BufferedOutputStream(new FileOutputStream(args[2])))) {
                Enumeration<? extends ZipEntry> en = zf.entries();
                while (en.hasMoreElements()) {
                    ZipEntry e = en.nextElement();
                    zout.putNextEntry(new ZipEntry(e.getName()));
                    try (InputStream in = zf.getInputStream(e)) {
                        in.transferTo(zout);
                    }
                    zout.closeEntry();
                }
            }
            System.out.println("normalized -> " + args[2]);
            return;
        }

        if (args.length == 3 && args[0].equals("--apply-branding")) {
            String inJar = args[1], outJar = args[2];
            int changed = 0;
            try (ZipInputStream zin = new ZipInputStream(new BufferedInputStream(new FileInputStream(inJar)));
                 ZipOutputStream zout = new ZipOutputStream(new BufferedOutputStream(new FileOutputStream(outJar)))) {
                ZipEntry e;
                byte[] buf = new byte[8192];
                while ((e = zin.getNextEntry()) != null) {
                    String name = e.getName();
                    ByteArrayOutputStream tmp = new ByteArrayOutputStream();
                    int n;
                    while ((n = zin.read(buf)) > 0) tmp.write(buf, 0, n);
                    byte[] data = tmp.toByteArray();
                    if (name.endsWith(".class")) {
                        byte[] replaced = replaceBranding(data);
                        if (replaced != null) { data = replaced; changed++; }
                    }
                    zout.putNextEntry(new ZipEntry(name));
                    zout.write(data);
                    zout.closeEntry();
                }
            }
            System.out.println("branded entries: " + changed + " -> " + outJar);
            return;
        }

        if (args.length >= 3 && args[0].equals("--scan-sub")) {
            String jar = args[1];
            String needle = args[2];
            try (ZipInputStream zin = new ZipInputStream(new FileInputStream(jar))) {
                ZipEntry e;
                while ((e = zin.getNextEntry()) != null) {
                    if (!e.getName().endsWith(".class")) continue;
                    byte[] data = zin.readAllBytes();
                    List<Utf8> found;
                    try { found = parse(data); } catch (IOException ex) { continue; }
                    LinkedHashSet<String> hits = new LinkedHashSet<>();
                    for (Utf8 u : found) if (u.value.contains(needle) && u.value.length() < 200) hits.add(u.value);
                    if (!hits.isEmpty()) System.out.println("  " + e.getName() + "  -> " + hits);
                }
            }
            return;
        }

        if (args.length >= 2 && args[0].equals("--scan")) {
            String jar = args[1];
            Set<String> needles = new HashSet<>(Arrays.asList(args).subList(2, args.length));
            try (ZipInputStream zin = new ZipInputStream(new FileInputStream(jar))) {
                ZipEntry e;
                while ((e = zin.getNextEntry()) != null) {
                    if (!e.getName().endsWith(".class")) continue;
                    byte[] data = zin.readAllBytes();
                    List<Utf8> found;
                    try { found = parse(data); } catch (IOException ex) { System.out.println("  !! parse error " + e.getName() + ": " + ex.getMessage()); continue; }
                    LinkedHashSet<String> hits = new LinkedHashSet<>();
                    for (Utf8 u : found) if (needles.contains(u.value)) hits.add(u.value);
                    if (!hits.isEmpty()) System.out.println("  " + e.getName() + "  -> " + hits);
                }
            }
            return;
        }

        if (args.length >= 4 && args[0].equals("--apply")) {
            String inJar = args[1], outJar = args[2], pairsPath = args[3];
            Map<String, String> pairs = readPairs(pairsPath);
            int changed = 0;
            try (ZipInputStream zin = new ZipInputStream(new BufferedInputStream(new FileInputStream(inJar)));
                 ZipOutputStream zout = new ZipOutputStream(new BufferedOutputStream(new FileOutputStream(outJar)))) {
                ZipEntry e;
                byte[] buf = new byte[8192];
                while ((e = zin.getNextEntry()) != null) {
                    String name = e.getName();
                    ZipEntry ne = new ZipEntry(name);
                    zout.putNextEntry(ne);
                    boolean isClass = name.endsWith(".class");
                    // Only lang/asset json is plain display text. fabric.mod.json and
                    // mixin configs contain class names that must not be rewritten.
                    boolean isAssetJson = name.startsWith("assets/") && name.endsWith(".json");
                    if (isClass || isAssetJson) {
                        ByteArrayOutputStream tmp = new ByteArrayOutputStream();
                        int n;
                        while ((n = zin.read(buf)) > 0) tmp.write(buf, 0, n);
                        byte[] data = tmp.toByteArray();
                        byte[] replaced = null;
                        if (name.endsWith(".class")) {
                            replaced = replace(data, pairs);
                            if (replaced != null) changed++;
                        } else {
                            String text = new String(data, StandardCharsets.UTF_8);
                            String newText = text;
                            for (Map.Entry<String, String> p : pairs.entrySet()) newText = newText.replace(p.getKey(), p.getValue());
                            if (!newText.equals(text)) { replaced = newText.getBytes(StandardCharsets.UTF_8); changed++; }
                        }
                        zout.write(replaced != null ? replaced : data);
                    } else {
                        int n;
                        while ((n = zin.read(buf)) > 0) zout.write(buf, 0, n);
                    }
                    zout.closeEntry();
                }
            }
            System.out.println("changed entries: " + changed + " -> " + outJar);
            return;
        }

        System.out.println("usage: --scan <jar> [values...] | --apply <in.jar> <out.jar> <pairs>");
        System.exit(2);
    }
}
