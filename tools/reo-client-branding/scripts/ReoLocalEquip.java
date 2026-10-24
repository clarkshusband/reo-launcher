package org.polyfrost.polyplus.client.cosmetics;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Objects;

import org.polyfrost.polyplus.client.network.http.responses.BodySlot;
import org.polyfrost.polyplus.client.network.http.responses.EquippedCosmetics;
import org.polyfrost.polyplus.client.network.http.responses.PartialEquippedCosmetics;

/**
 * Runtime state for the Reo "equip even when the backend rejects it" patch.
 *
 * <p>The local player's equipped cosmetics are held in
 * {@code CosmeticCatalog.localEquipped}, which is rebuilt from the HTTP
 * {@code /cosmetics/player} response by {@code refreshPlayer}.  Anything that
 * only mutated the catalog would therefore be wiped by the refresh that runs
 * immediately after an equip.  This class records the slots the client asked
 * for and lets the patch merge them over the server response until a request
 * for that slot succeeds.</p>
 *
 * <p>A {@code null} value means "clear this slot".</p>
 */
public final class ReoLocalEquip {
    private static final Map<BodySlot, Integer> PENDING = new LinkedHashMap<>();

    private ReoLocalEquip() {
    }

    /** Records the slots a setEquipped call is trying to apply. */
    public static void record(PartialEquippedCosmetics partial) {
        Map<BodySlot, Integer> incoming = equippedOf(partial);
        if (incoming == null) {
            return;
        }
        synchronized (PENDING) {
            for (Map.Entry<BodySlot, Integer> entry : incoming.entrySet()) {
                PENDING.put(entry.getKey(), entry.getValue());
            }
        }
    }

    /**
     * Drops pending overrides for slots the backend accepted.  The following
     * refresh can then use the server state for those slots unchanged.
     */
    public static void markServerSuccess(PartialEquippedCosmetics partial) {
        Map<BodySlot, Integer> accepted = equippedOf(partial);
        if (accepted == null) {
            return;
        }
        synchronized (PENDING) {
            for (Map.Entry<BodySlot, Integer> entry : accepted.entrySet()) {
                BodySlot slot = entry.getKey();
                if (PENDING.containsKey(slot)
                        && Objects.equals(PENDING.get(slot), entry.getValue())) {
                    PENDING.remove(slot);
                }
            }
        }
    }

    /**
     * Overlays pending local changes on a fresh server snapshot.  Called from
     * {@code CosmeticCatalog.refreshPlayer} right before {@code localEquipped}
     * is assigned.
     */
    public static EquippedCosmetics mergePending(EquippedCosmetics server) {
        Map<BodySlot, Integer> base = server == null ? null : server.getEquipped();
        Map<BodySlot, Integer> merged;
        synchronized (PENDING) {
            if (PENDING.isEmpty()) {
                return server;
            }
            merged = new LinkedHashMap<>();
            if (base != null) {
                merged.putAll(base);
            }
            for (Map.Entry<BodySlot, Integer> entry : PENDING.entrySet()) {
                if (entry.getValue() == null) {
                    merged.remove(entry.getKey());
                } else {
                    merged.put(entry.getKey(), entry.getValue());
                }
            }
        }
        return new EquippedCosmetics(merged);
    }

    /** Clears all overrides; wired into CosmeticCatalog.reset(). */
    public static void clear() {
        synchronized (PENDING) {
            PENDING.clear();
        }
    }

    private static Map<BodySlot, Integer> equippedOf(PartialEquippedCosmetics partial) {
        return partial == null ? null : partial.getEquipped();
    }
}
