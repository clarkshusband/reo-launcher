import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Enumeration;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import java.util.zip.ZipOutputStream;

/**
 * Reo cosmetics unlock.
 *
 * <ul>
 *   <li>CosmeticCatalog.ownedCosmeticIds()/ownedEmoteIds()/ownedIds() report
 *       every loaded catalog definition as owned, so the store and wardrobe
 *       treat all cosmetics as unlocked.</li>
 *   <li>CosmeticStoreInfo and CartEntry price getters return 0, so the store
 *       UI renders "FREE" instead of a price.</li>
 * </ul>
 */
public final class ReoCosmeticsFree {
    private static final String CATALOG = "org/polyfrost/polyplus/client/cosmetics/CosmeticCatalog";
    private static final String STORE_INFO = "org/polyfrost/polyplus/client/network/http/responses/CosmeticStoreInfo";
    private static final String CART_ENTRY = "org/polyfrost/polyplus/client/gui/CartEntry";
    private static final String CHM = "java/util/concurrent/ConcurrentHashMap";
    private static final String CHM_DESC = "Ljava/util/concurrent/ConcurrentHashMap;";

    public static void main(String[] args) throws Exception {
        if (args.length != 2) {
            System.err.println("usage: ReoCosmeticsFree <in.jar> <out.jar>");
            System.exit(2);
        }
        Path in = Paths.get(args[0]);
        Path out = Paths.get(args[1]);
        int classes = 0;
        try (ZipFile zip = new ZipFile(in.toFile());
             ZipOutputStream zos = new ZipOutputStream(Files.newOutputStream(out))) {
            Enumeration<? extends ZipEntry> entries = zip.entries();
            while (entries.hasMoreElements()) {
                ZipEntry entry = entries.nextElement();
                byte[] data;
                try (InputStream is = zip.getInputStream(entry)) {
                    data = is.readAllBytes();
                }
                String name = entry.getName();
                if (name.equals(CATALOG + ".class")) {
                    data = patchCatalog(data);
                    classes++;
                } else if (name.equals(STORE_INFO + ".class") || name.equals(CART_ENTRY + ".class")) {
                    data = patchPriceGetters(data);
                    classes++;
                }
                ZipEntry copy = new ZipEntry(name);
                copy.setTime(entry.getTime());
                zos.putNextEntry(copy);
                zos.write(data);
                zos.closeEntry();
            }
        }
        System.out.println("ReoCosmeticsFree: patched " + classes + " classes -> " + out);
    }

    private static byte[] patchCatalog(byte[] data) {
        ClassReader cr = new ClassReader(data);
        ClassWriter cw = new ClassWriter(cr, ClassWriter.COMPUTE_MAXS);
        cr.accept(new ClassVisitor(Opcodes.ASM9, cw) {
            @Override
            public MethodVisitor visitMethod(int access, String name, String desc, String sig, String[] ex) {
                if (!desc.equals("()Ljava/util/Set;")) {
                    return super.visitMethod(access, name, desc, sig, ex);
                }
                if (name.equals("ownedCosmeticIds")) {
                    MethodVisitor mv = super.visitMethod(access, name, desc, sig, ex);
                    emitKeyset(mv, "cosmeticDefinitions");
                    return null;
                }
                if (name.equals("ownedEmoteIds")) {
                    MethodVisitor mv = super.visitMethod(access, name, desc, sig, ex);
                    emitKeyset(mv, "emoteDefinitions");
                    return null;
                }
                if (name.equals("ownedIds")) {
                    MethodVisitor mv = super.visitMethod(access, name, desc, sig, ex);
                    mv.visitCode();
                    mv.visitFieldInsn(Opcodes.GETSTATIC, CATALOG, "cosmeticDefinitions", CHM_DESC);
                    mv.visitMethodInsn(Opcodes.INVOKEVIRTUAL, CHM, "keySet", "()Ljava/util/Set;", false);
                    mv.visitFieldInsn(Opcodes.GETSTATIC, CATALOG, "emoteDefinitions", CHM_DESC);
                    mv.visitMethodInsn(Opcodes.INVOKEVIRTUAL, CHM, "keySet", "()Ljava/util/Set;", false);
                    mv.visitMethodInsn(Opcodes.INVOKESTATIC, "kotlin/collections/SetsKt", "plus",
                            "(Ljava/util/Set;Ljava/lang/Iterable;)Ljava/util/Set;", false);
                    mv.visitInsn(Opcodes.ARETURN);
                    mv.visitMaxs(2, 1);
                    mv.visitEnd();
                    return null;
                }
                return super.visitMethod(access, name, desc, sig, ex);
            }
        }, 0);
        return cw.toByteArray();
    }

    private static void emitKeyset(MethodVisitor mv, String field) {
        mv.visitCode();
        mv.visitFieldInsn(Opcodes.GETSTATIC, CATALOG, field, CHM_DESC);
        mv.visitMethodInsn(Opcodes.INVOKEVIRTUAL, CHM, "keySet", "()Ljava/util/Set;", false);
        mv.visitInsn(Opcodes.ARETURN);
        mv.visitMaxs(1, 1);
        mv.visitEnd();
    }

    private static byte[] patchPriceGetters(byte[] data) {
        ClassReader cr = new ClassReader(data);
        ClassWriter cw = new ClassWriter(cr, ClassWriter.COMPUTE_MAXS);
        cr.accept(new ClassVisitor(Opcodes.ASM9, cw) {
            @Override
            public MethodVisitor visitMethod(int access, String name, String desc, String sig, String[] ex) {
                if (!desc.equals("()Ljava/lang/Float;")
                        || !(name.equals("getBasePrice") || name.equals("getFinalPrice"))) {
                    return super.visitMethod(access, name, desc, sig, ex);
                }
                MethodVisitor mv = super.visitMethod(access, name, desc, sig, ex);
                mv.visitCode();
                mv.visitInsn(Opcodes.FCONST_0);
                mv.visitMethodInsn(Opcodes.INVOKESTATIC, "java/lang/Float", "valueOf", "(F)Ljava/lang/Float;", false);
                mv.visitInsn(Opcodes.ARETURN);
                mv.visitMaxs(1, 1);
                mv.visitEnd();
                return null;
            }
        }, 0);
        return cw.toByteArray();
    }
}
