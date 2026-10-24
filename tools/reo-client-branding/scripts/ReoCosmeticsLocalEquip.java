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
 * Reo cosmetic local-equip fix.
 *
 * <p>The store unlock makes every catalog entry look owned to the client, but
 * the backend still rejects equipment the account does not own.  The original
 * flow is:</p>
 *
 * <pre>
 * CosmeticService.equip
 *   -&gt; setEquippedAndSync
 *      -&gt; CosmeticCatalog.setEquipped (HTTP PUT /cosmetics/player)
 *         -&gt; 400 "not owned" -&gt; Result.failure
 *      -&gt; setEquippedAndSync returns early (no websocket packet, no local sync)
 * </pre>
 *
 * <p>This patcher records the attempted equipment in
 * {@code ReoLocalEquip}, makes the HTTP failure non-fatal, ignores a failed
 * realtime packet, and merges the recorded slots over the server snapshot in
 * {@code refreshPlayer}.  As a result the wardrobe/visual state always applies
 * locally, while a successful backend request still behaves normally (the
 * pending override is dropped and the server state wins).</p>
 */
public final class ReoCosmeticsLocalEquip {
    private static final String CATALOG =
            "org/polyfrost/polyplus/client/cosmetics/CosmeticCatalog";
    private static final String SERVICE =
            "org/polyfrost/polyplus/client/cosmetics/CosmeticService";
    private static final String HELPER =
            "org/polyfrost/polyplus/client/cosmetics/ReoLocalEquip";
    private static final String HELPER_CLASS = HELPER + ".class";

    private static final String PARTIAL =
            "org/polyfrost/polyplus/client/network/http/responses/PartialEquippedCosmetics";
    private static final String PARTIAL_DESC = "L" + PARTIAL + ";";
    private static final String EQUIPPED =
            "org/polyfrost/polyplus/client/network/http/responses/EquippedCosmetics";
    private static final String EQUIPPED_DESC = "L" + EQUIPPED + ";";
    private static final String BODY_SLOT =
            "org/polyfrost/polyplus/client/network/http/responses/BodySlot";
    private static final String CONTINUATION = "kotlin/coroutines/Continuation";

    private static final String SET_EQUIPPED_DESC =
            "(" + PARTIAL_DESC + "L" + CONTINUATION + ";)Ljava/lang/Object;";
    private static final String REFRESH_PLAYER_DESC =
            "(L" + CONTINUATION + ";)Ljava/lang/Object;";
    private static final String SET_EQUIPPED_AND_SYNC_DESC =
            "(L" + BODY_SLOT + ";Ljava/lang/Integer;L" + CONTINUATION + ";)Ljava/lang/Object;";

    public static void main(String[] args) throws Exception {
        if (args.length < 2 || args.length > 3) {
            System.err.println("usage: ReoCosmeticsLocalEquip <in.jar> <out.jar> [ReoLocalEquip.class]");
            System.exit(2);
        }
        Path in = Paths.get(args[0]);
        Path out = Paths.get(args[1]);
        byte[] helper = readHelper(args.length == 3 ? Paths.get(args[2]) : null);

        int catalogPatched = 0;
        int servicePatched = 0;
        boolean helperWritten = false;
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
                    catalogPatched++;
                } else if (name.equals(SERVICE + ".class")) {
                    data = patchService(data);
                    servicePatched++;
                }
                if (name.equals(HELPER_CLASS)) {
                    data = helper;
                    helperWritten = true;
                }
                ZipEntry copy = new ZipEntry(name);
                copy.setTime(entry.getTime());
                zos.putNextEntry(copy);
                zos.write(data);
                zos.closeEntry();
            }
            if (!helperWritten) {
                ZipEntry copy = new ZipEntry(HELPER_CLASS);
                copy.setTime(System.currentTimeMillis());
                zos.putNextEntry(copy);
                zos.write(helper);
                zos.closeEntry();
            }
        }
        if (catalogPatched != 1 || servicePatched != 1) {
            throw new IllegalStateException("expected one Catalog and one Service class, got "
                    + catalogPatched + " and " + servicePatched);
        }
        System.out.println("ReoCosmeticsLocalEquip: patched " + (catalogPatched + servicePatched)
                + " classes, added " + HELPER_CLASS + " -> " + out);
    }

    private static byte[] readHelper(Path explicit) throws Exception {
        if (explicit != null) {
            return Files.readAllBytes(explicit);
        }
        try (InputStream is = ReoCosmeticsLocalEquip.class.getResourceAsStream("/" + HELPER_CLASS)) {
            if (is == null) {
                throw new IllegalStateException("ReoLocalEquip.class not found next to patcher; pass it as arg 3");
            }
            return is.readAllBytes();
        }
    }

    private static byte[] patchCatalog(byte[] data) {
        ClassReader cr = new ClassReader(data);
        ClassWriter cw = new ClassWriter(cr, ClassWriter.COMPUTE_MAXS);
        final boolean[] found = new boolean[3];
        cr.accept(new ClassVisitor(Opcodes.ASM9, cw) {
            @Override
            public MethodVisitor visitMethod(int access, String name, String desc,
                                             String signature, String[] exceptions) {
                MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
                if (name.startsWith("setEquipped-") && desc.equals(SET_EQUIPPED_DESC)) {
                    found[0] = true;
                    return new SetEquippedVisitor(mv);
                }
                if (name.equals("refreshPlayer") && desc.equals(REFRESH_PLAYER_DESC)) {
                    found[1] = true;
                    return new RefreshPlayerVisitor(mv);
                }
                if (name.equals("reset") && desc.equals("()V")) {
                    found[2] = true;
                    return new ResetVisitor(mv);
                }
                return mv;
            }
        }, 0);
        for (int i = 0; i < found.length; i++) {
            if (!found[i]) {
                throw new IllegalStateException("CosmeticCatalog patch target #" + i + " not found");
            }
        }
        return cw.toByteArray();
    }

    private static byte[] patchService(byte[] data) {
        ClassReader cr = new ClassReader(data);
        ClassWriter cw = new ClassWriter(cr, ClassWriter.COMPUTE_MAXS);
        final boolean[] found = new boolean[1];
        cr.accept(new ClassVisitor(Opcodes.ASM9, cw) {
            @Override
            public MethodVisitor visitMethod(int access, String name, String desc,
                                             String signature, String[] exceptions) {
                MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
                if (name.startsWith("setEquippedAndSync") && desc.equals(SET_EQUIPPED_AND_SYNC_DESC)) {
                    found[0] = true;
                    return new SetEquippedAndSyncVisitor(mv);
                }
                return mv;
            }
        }, 0);
        if (!found[0]) {
            throw new IllegalStateException("CosmeticService.setEquippedAndSync not found");
        }
        return cw.toByteArray();
    }

    /** Records the request and turns the final failure return into success. */
    private static final class SetEquippedVisitor extends MethodVisitor {
        private boolean successMarked;
        private boolean failureLogged;
        private boolean failureReplaced;

        SetEquippedVisitor(MethodVisitor mv) {
            super(Opcodes.ASM9, mv);
        }

        @Override
        public void visitCode() {
            super.visitCode();
            super.visitVarInsn(Opcodes.ALOAD, 1);
            super.visitMethodInsn(Opcodes.INVOKESTATIC, HELPER, "record",
                    "(" + PARTIAL_DESC + ")V", false);
        }

        @Override
        public void visitMethodInsn(int opcode, String owner, String name, String desc, boolean itf) {
            super.visitMethodInsn(opcode, owner, name, desc, itf);
            if (opcode == Opcodes.INVOKEINTERFACE
                    && owner.equals("org/apache/logging/log4j/Logger")
                    && name.equals("error")
                    && desc.equals("(Ljava/lang/String;Ljava/lang/Throwable;)V")) {
                failureLogged = true;
            }
        }

        @Override
        public void visitFieldInsn(int opcode, String owner, String name, String desc) {
            if (!successMarked && opcode == Opcodes.GETSTATIC
                    && owner.equals("kotlin/Unit") && name.equals("INSTANCE")) {
                super.visitVarInsn(Opcodes.ALOAD, 1);
                super.visitMethodInsn(Opcodes.INVOKESTATIC, HELPER, "markServerSuccess",
                        "(" + PARTIAL_DESC + ")V", false);
                successMarked = true;
            }
            super.visitFieldInsn(opcode, owner, name, desc);
        }

        @Override
        public void visitVarInsn(int opcode, int var) {
            if (failureLogged && !failureReplaced && opcode == Opcodes.ALOAD && var == 3) {
                // Replace the failure Result with Result.success(Unit).
                super.visitFieldInsn(Opcodes.GETSTATIC, "kotlin/Unit", "INSTANCE", "Lkotlin/Unit;");
                super.visitMethodInsn(Opcodes.INVOKESTATIC, "kotlin/Result", "constructor-impl",
                        "(Ljava/lang/Object;)Ljava/lang/Object;", false);
                failureReplaced = true;
                return;
            }
            super.visitVarInsn(opcode, var);
        }

        @Override
        public void visitEnd() {
            if (!successMarked || !failureReplaced) {
                throw new IllegalStateException("setEquipped structure changed (success="
                        + successMarked + ", failure=" + failureReplaced + ")");
            }
            super.visitEnd();
        }
    }

    /** Merges pending local choices over the server snapshot. */
    private static final class RefreshPlayerVisitor extends MethodVisitor {
        private boolean seenServerSnapshot;
        private boolean merged;

        RefreshPlayerVisitor(MethodVisitor mv) {
            super(Opcodes.ASM9, mv);
        }

        @Override
        public void visitMethodInsn(int opcode, String owner, String name, String desc, boolean itf) {
            super.visitMethodInsn(opcode, owner, name, desc, itf);
            if (opcode == Opcodes.INVOKESPECIAL
                    && owner.equals(EQUIPPED) && name.equals("<init>")
                    && desc.equals("(Ljava/util/Map;)V")) {
                seenServerSnapshot = true;
            }
        }

        @Override
        public void visitFieldInsn(int opcode, String owner, String name, String desc) {
            if (seenServerSnapshot && opcode == Opcodes.PUTSTATIC
                    && owner.equals(CATALOG) && name.equals("localEquipped")
                    && desc.equals(EQUIPPED_DESC)) {
                super.visitMethodInsn(Opcodes.INVOKESTATIC, HELPER, "mergePending",
                        "(" + EQUIPPED_DESC + ")" + EQUIPPED_DESC, false);
                seenServerSnapshot = false;
                merged = true;
            }
            super.visitFieldInsn(opcode, owner, name, desc);
        }

        @Override
        public void visitEnd() {
            if (!merged) {
                throw new IllegalStateException("refreshPlayer localEquipped assignment not found");
            }
            super.visitEnd();
        }
    }

    /** Clears overrides when the catalog is reset (disconnect/server change). */
    private static final class ResetVisitor extends MethodVisitor {
        ResetVisitor(MethodVisitor mv) {
            super(Opcodes.ASM9, mv);
        }

        @Override
        public void visitCode() {
            super.visitCode();
            super.visitMethodInsn(Opcodes.INVOKESTATIC, HELPER, "clear", "()V", false);
        }
    }

    /** Makes both Result.isFailure checks non-fatal. */
    private static final class SetEquippedAndSyncVisitor extends MethodVisitor {
        private boolean afterIsFailure;
        private int patched;

        SetEquippedAndSyncVisitor(MethodVisitor mv) {
            super(Opcodes.ASM9, mv);
        }

        @Override
        public void visitMethodInsn(int opcode, String owner, String name, String desc, boolean itf) {
            super.visitMethodInsn(opcode, owner, name, desc, itf);
            if (opcode == Opcodes.INVOKESTATIC && owner.equals("kotlin/Result")
                    && name.equals("isFailure-impl") && desc.equals("(Ljava/lang/Object;)Z")) {
                afterIsFailure = true;
            }
        }

        @Override
        public void visitJumpInsn(int opcode, org.objectweb.asm.Label label) {
            if (afterIsFailure && opcode == Opcodes.IFEQ) {
                // Drop the isFailure boolean and branch unconditionally.  Keep an
                // IFEQ (with a constant 0) instead of GOTO so the fall-through
                // failure return stays type-reachable and needs no new stack map
                // frame; a GOTO would make it dead code and fail verification.
                super.visitInsn(Opcodes.POP);
                super.visitInsn(Opcodes.ICONST_0);
                super.visitJumpInsn(Opcodes.IFEQ, label);
                afterIsFailure = false;
                patched++;
                return;
            }
            super.visitJumpInsn(opcode, label);
        }

        @Override
        public void visitEnd() {
            if (patched != 2) {
                throw new IllegalStateException("expected 2 backend/websocket failure checks, patched " + patched);
            }
            super.visitEnd();
        }
    }
}
