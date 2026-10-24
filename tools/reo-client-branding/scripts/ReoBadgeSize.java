import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.ClassNode;
import org.objectweb.asm.tree.FieldNode;
import org.objectweb.asm.tree.IntInsnNode;
import org.objectweb.asm.tree.MethodNode;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.Enumeration;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;
import java.util.zip.CRC32;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import java.util.zip.ZipOutputStream;

/**
 * Installs the ReoPlus/PolyPlus chrome R badge without changing the mod's
 * public API.
 *
 * - PolyPlusBadge.blitTab: 9x8 tab-list texture -> 16x9 tab-list texture
 * - PolyPlusBadge font constants: 12x12 glyph (24x24 source texture)
 * - MixinPlayerTabOverlay badge offset: 12 -> 19 to match the wider tab badge
 *
 * The patch is idempotent: it also accepts jars that are already at 16x9/12.
 *
 * Usage: java ReoBadgeSize <in.jar> <out.jar> <asset-dir>
 * The asset dir must contain badge.png, badge.png.mcmeta, badge_font.png,
 * badge_font.png.mcmeta and badge.json.
 */
public final class ReoBadgeSize {
    private static final String BADGE_CLASS = "org/polyfrost/polyplus/client/PolyPlusBadge.class";
    private static final String TAB_CLASS = "org/polyfrost/polyplus/mixin/client/MixinPlayerTabOverlay.class";

    private static final Map<String, Integer> FIELD_VALUES = new HashMap<>();
    static {
        FIELD_VALUES.put("TEX_W", 64);
        FIELD_VALUES.put("TEX_H", 36);
        FIELD_VALUES.put("TEXT_H", 12);
        FIELD_VALUES.put("BADGE_H", 9);
        FIELD_VALUES.put("BADGE_W", 16);
        FIELD_VALUES.put("BADGE_ADVANCE", 15);
    }

    private static final Map<String, Path> ASSETS = new HashMap<>();
    static {
        ASSETS.put("assets/polyplus/textures/badge.png", Path.of("badge.png"));
        ASSETS.put("assets/polyplus/textures/badge.png.mcmeta", Path.of("badge.png.mcmeta"));
        ASSETS.put("assets/polyplus/textures/badge_font.png", Path.of("badge_font.png"));
        ASSETS.put("assets/polyplus/textures/badge_font.png.mcmeta", Path.of("badge_font.png.mcmeta"));
        ASSETS.put("assets/polyplus/font/badge.json", Path.of("badge.json"));
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 3) {
            throw new IllegalArgumentException("usage: ReoBadgeSize <in.jar> <out.jar> <asset-dir>");
        }
        Path in = Path.of(args[0]).toAbsolutePath();
        Path out = Path.of(args[1]).toAbsolutePath();
        Path assetDir = Path.of(args[2]).toAbsolutePath();
        if (!Files.isRegularFile(in)) throw new IOException("missing input jar: " + in);
        if (!Files.isDirectory(assetDir)) throw new IOException("missing asset dir: " + assetDir);

        Map<String, byte[]> assets = new HashMap<>();
        for (Map.Entry<String, Path> e : ASSETS.entrySet()) {
            Path p = assetDir.resolve(e.getValue());
            if (!Files.isRegularFile(p)) throw new IOException("missing asset: " + p);
            assets.put(e.getKey(), Files.readAllBytes(p));
        }

        Set<String> seen = new HashSet<>();
        int replaced = 0;
        int added = 0;
        try (ZipFile zf = new ZipFile(in.toFile());
             ZipOutputStream zo = new ZipOutputStream(Files.newOutputStream(out))) {
            Enumeration<? extends ZipEntry> en = zf.entries();
            while (en.hasMoreElements()) {
                ZipEntry entry = en.nextElement();
                String name = entry.getName();
                byte[] data;
                if (assets.containsKey(name)) {
                    data = assets.get(name);
                    replaced++;
                } else {
                    try (InputStream is = zf.getInputStream(entry)) {
                        data = is.readAllBytes();
                    }
                    if (name.equals(BADGE_CLASS)) data = patchBadge(data);
                    else if (name.equals(TAB_CLASS)) data = patchTabOffset(data);
                }
                seen.add(name);

                ZipEntry copy = new ZipEntry(name);
                copy.setTime(entry.getTime());
                if (entry.isDirectory()) {
                    copy.setMethod(ZipEntry.STORED);
                    copy.setSize(0);
                    copy.setCrc(0);
                } else {
                    copy.setMethod(ZipEntry.DEFLATED);
                }
                zo.putNextEntry(copy);
                if (!entry.isDirectory()) zo.write(data);
                zo.closeEntry();
            }

            for (Map.Entry<String, byte[]> e : assets.entrySet()) {
                if (seen.contains(e.getKey())) continue;
                ZipEntry copy = new ZipEntry(e.getKey());
                copy.setTime(System.currentTimeMillis());
                zo.putNextEntry(copy);
                zo.write(e.getValue());
                zo.closeEntry();
                added++;
            }
        }
        System.out.println("patched " + in + " -> " + out);
        System.out.println("replaced assets=" + replaced + " added assets=" + added);
    }

    private static byte[] patchBadge(byte[] bytes) {
        ClassNode cn = read(bytes);
        boolean foundBlit = false;
        Set<String> foundFields = new HashSet<>();
        for (FieldNode f : cn.fields) {
            Integer value = FIELD_VALUES.get(f.name);
            if (value != null) {
                f.value = value;
                foundFields.add(f.name);
            }
        }
        for (MethodNode m : cn.methods) {
            if (!m.name.equals("blitTab")) {
                continue;
            }
            java.util.List<IntInsnNode> ints = new java.util.ArrayList<>();
            for (AbstractInsnNode insn : m.instructions) {
                if (insn instanceof IntInsnNode) ints.add((IntInsnNode) insn);
            }
            int[] from = {9, 8, 48, 45, 48, 45};
            int[] to = {16, 9, 64, 36, 64, 36};
            for (int i = 0; i + from.length <= ints.size(); i++) {
                boolean match = true;
                for (int j = 0; j < from.length; j++) {
                    if (ints.get(i + j).operand != from[j]) { match = false; break; }
                }
                if (match) {
                    for (int j = 0; j < to.length; j++) ints.get(i + j).operand = to[j];
                    foundBlit = true;
                    break;
                }
            }
            if (!foundBlit) {
                for (int i = 0; i + to.length <= ints.size(); i++) {
                    boolean match = true;
                    for (int j = 0; j < to.length; j++) {
                        if (ints.get(i + j).operand != to[j]) { match = false; break; }
                    }
                    if (match) { foundBlit = true; break; }
                }
            }
        }
        if (!foundBlit) throw new IllegalStateException("blitTab 9x8/48x45 or 16x9/64x36 sequence not found");
        if (foundFields.size() != FIELD_VALUES.size()) {
            throw new IllegalStateException("badge constants not all found: " + foundFields);
        }
        return write(cn);
    }

    private static byte[] patchTabOffset(byte[] bytes) {
        ClassNode cn = read(bytes);
        int already = 0;
        int patched = 0;
        for (MethodNode m : cn.methods) {
            if (!m.name.equals("polyplus$badgeOffset")) continue;
            for (AbstractInsnNode insn : m.instructions) {
                if (!(insn instanceof IntInsnNode)) continue;
                int value = ((IntInsnNode) insn).operand;
                if (value == 19) already++;
                else if (value == 12) {
                    ((IntInsnNode) insn).operand = 19;
                    patched++;
                }
            }
        }
        if (already == 1 && patched == 0) return bytes;
        if (patched != 1) throw new IllegalStateException("expected one 12px tab offset or one 19px offset, already=" + already + " patched=" + patched);
        return write(cn);
    }

    private static boolean isInt(AbstractInsnNode insn, int value) {
        return insn instanceof IntInsnNode && ((IntInsnNode) insn).operand == value;
    }

    private static ClassNode read(byte[] bytes) {
        ClassNode cn = new ClassNode();
        new ClassReader(bytes).accept(cn, 0);
        return cn;
    }

    private static byte[] write(ClassNode cn) {
        ClassWriter cw = new ClassWriter(0);
        cn.accept(cw);
        return cw.toByteArray();
    }
}
