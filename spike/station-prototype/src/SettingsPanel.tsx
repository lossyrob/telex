import { useState } from "react";
import { Modal, TierMark, useStation } from "./bits";
import { clock, relTime, TIER_LABEL, TIER_MEANING } from "./model";
import type { ChannelPolicy, Settings, Tier } from "./types";

const TIER_KEY: Record<Tier, "blocked" | "needs_reply" | "routine" | "fyi"> = {
  blocked: "blocked",
  "needs-reply": "needs_reply",
  routine: "routine",
  fyi: "fyi",
};

const CHANNELS: { key: keyof ChannelPolicy; label: string; help: string }[] = [
  { key: "banner", label: "Banner", help: "A macOS notification banner." },
  { key: "sound", label: "Sound", help: "Play a sound with the banner." },
  { key: "demand", label: "Demand attention", help: "Bounce the dock icon and keep a red strip in the window until you look." },
  { key: "badge", label: "Badge", help: "Count in the dock and menu bar. The quiet channel." },
];

function minutes(value: number): string {
  const h = String(Math.floor(value / 60)).padStart(2, "0");
  const m = String(value % 60).padStart(2, "0");
  return `${h}:${m}`;
}

function parseMinutes(value: string): number {
  const [h, m] = value.split(":").map(Number);
  return (h || 0) * 60 + (m || 0);
}

function Notifications() {
  const { snap, act } = useStation();
  const s = snap.settings;
  const update = (next: Settings) => void act("settings_update", { settings: next });
  const setChannel = (tier: Tier, key: keyof ChannelPolicy, value: boolean | number) =>
    update({ ...s, [TIER_KEY[tier]]: { ...s[TIER_KEY[tier]], [key]: value } });
  return (
    <div>
      <p className="lede">
        Priority comes from the agent’s Telex attention level, never from message text. These settings decide how each level reaches you on this
        Mac. Projects, agents and threads can override them.
      </p>
      <table className="matrix">
        <thead>
          <tr>
            <th>When a message is</th>
            {CHANNELS.map((c) => (
              <th key={c.key} title={c.help}>{c.label}</th>
            ))}
            <th title="Notify again while still unanswered">Remind</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {(["blocked", "needs-reply", "routine", "fyi"] as Tier[]).map((tier) => {
            const p = s[TIER_KEY[tier]];
            return (
              <tr key={tier}>
                <td>
                  <TierMark tier={tier} label />
                  <div className="muted small">{TIER_MEANING[tier]}</div>
                </td>
                {CHANNELS.map((c) => (
                  <td key={c.key} className="center">
                    <input
                      type="checkbox"
                      aria-label={`${TIER_LABEL[tier]} ${c.label}`}
                      checked={p[c.key] as boolean}
                      disabled={(c.key === "sound" || c.key === "demand") && !p.banner}
                      onChange={(e) => setChannel(tier, c.key, e.target.checked)}
                    />
                  </td>
                ))}
                <td>
                  <select value={p.remind_after_min} onChange={(e) => setChannel(tier, "remind_after_min", Number(e.target.value))}>
                    <option value={0}>never</option>
                    <option value={5}>every 5 min</option>
                    <option value={10}>every 10 min</option>
                    <option value={30}>every 30 min</option>
                    <option value={60}>hourly</option>
                  </select>
                </td>
                <td>
                  <button type="button" className="btn btn-mini" onClick={() => void act("notification_test", { tier })}>
                    Test
                  </button>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>

      <h3 className="sub-head">Quiet hours</h3>
      <div className="form-row">
        <label className="check">
          <input type="checkbox" checked={s.quiet_hours.enabled} onChange={(e) => update({ ...s, quiet_hours: { ...s.quiet_hours, enabled: e.target.checked } })} />
          <span>No banners between</span>
        </label>
        <input type="time" value={minutes(s.quiet_hours.start_min)} onChange={(e) => update({ ...s, quiet_hours: { ...s.quiet_hours, start_min: parseMinutes(e.target.value) } })} />
        <span>and</span>
        <input type="time" value={minutes(s.quiet_hours.end_min)} onChange={(e) => update({ ...s, quiet_hours: { ...s.quiet_hours, end_min: parseMinutes(e.target.value) } })} />
        <label className="check">
          <input type="checkbox" checked={s.quiet_hours.allow_blocked} onChange={(e) => update({ ...s, quiet_hours: { ...s.quiet_hours, allow_blocked: e.target.checked } })} />
          <span>except Blocked</span>
        </label>
      </div>

      <h3 className="sub-head">Bursts</h3>
      <div className="form-row">
        <span>Fold banners that arrive within</span>
        <select value={s.coalesce_window_s} onChange={(e) => update({ ...s, coalesce_window_s: Number(e.target.value) })}>
          <option value={0}>off</option>
          <option value={8}>8 seconds</option>
          <option value={30}>30 seconds</option>
          <option value={120}>2 minutes</option>
        </select>
        <span>into one summary. Blocked is never folded.</span>
      </div>

      <h3 className="sub-head">Other</h3>
      <div className="form-col">
        <label className="check">
          <input type="checkbox" checked={s.read_receipts} onChange={(e) => update({ ...s, read_receipts: e.target.checked })} />
          <span>Tell agents when I have seen a message that asks for an outcome</span>
        </label>
        {snap.station.port === "cli" && <div className="muted small indent">Unavailable over the CLI connection used on macOS today.</div>}
        <label className="check">
          <input type="checkbox" checked={s.use_project_hints} onChange={(e) => update({ ...s, use_project_hints: e.target.checked })} />
          <span>File agents by their own project hint when it names a project I already have</span>
        </label>
      </div>
    </div>
  );
}

function Health() {
  const { snap, act } = useStation();
  const [address, setAddress] = useState("");
  const st = snap.station;
  return (
    <div>
      <h3 className="sub-head">Connection</h3>
      <dl className="facts">
        <div className="fact"><dt>Telex</dt><dd>{st.connected ? "Connected" : `Not connected${st.connect_error ? `: ${st.connect_error}` : ""}`}</dd></div>
        <div className="fact"><dt>Build</dt><dd>{st.telex_build ?? "unknown"}</dd></div>
        <div className="fact"><dt>Backend</dt><dd>{st.backend}{st.sandbox ? " (isolated sandbox)" : ""}</dd></div>
        <div className="fact"><dt>Store</dt><dd className="mono">{st.store_id || "n/a"}</dd></div>
        <div className="fact"><dt>Connection type</dt><dd>{st.port === "cli" ? "telex CLI session" : "Application Client"}</dd></div>
      </dl>
      {st.port_note && <div className="callout">{st.port_note}</div>}

      <h3 className="sub-head">Addresses this Station attends</h3>
      <p className="muted small">Agents reach you by sending ordinary Telex messages to these addresses. Attending an address says a Station is running, not that you are at your desk.</p>
      {snap.health.map((h) => {
        const ok = h.registered && h.receive_ready && !h.last_error && !h.degraded;
        return (
          <div className="address" key={h.address}>
            <span className={`health-dot ${ok ? "ok" : "bad"}`} />
            <div className="address-main">
              <div className="mono">{h.address}</div>
              <div className="muted small">
                {h.registered ? "attached" : "not attached"} · {h.receive_ready ? "receiving" : "not receiving"} · {h.sender_ready ? "can send" : "cannot send"}
                {h.pending_unconsumed > 0 && ` · ${h.pending_unconsumed} waiting to be taken in`}
                {h.last_receive_ms && ` · last delivery ${relTime(h.last_receive_ms, snap.now_ms)} ago`}
                {h.principal && ` · principal ${h.principal} (${h.principal_verification})`}
              </div>
              {h.last_error && <div className="receipt-error">{h.last_error}</div>}
            </div>
            <button
              type="button"
              className="btn btn-mini danger"
              onClick={() => {
                if (window.confirm(`Stop attending ${h.address}? Messages sent there will queue until something attends it again.`)) {
                  void act("address_remove", { address: h.address });
                }
              }}
            >
              Stop attending
            </button>
          </div>
        );
      })}
      <div className="form-row">
        <input placeholder="another address, e.g. oncall:rob" value={address} onChange={(e) => setAddress(e.target.value)} />
        <button
          type="button"
          className="btn btn-primary"
          disabled={!address.trim()}
          onClick={() => {
            void act("address_add", { address: address.trim() });
            setAddress("");
          }}
        >
          Attend
        </button>
      </div>
    </div>
  );
}

function Activity() {
  const { snap, act, openThread } = useStation();
  return (
    <div>
      <p className="lede">Every notification decision, including the ones that stayed quiet and why. “Shown” means Station submitted it to macOS, not that you saw it.</p>
      <div className="form-row">
        <span className="spacer" />
        <button type="button" className="btn btn-mini" onClick={() => void act("notifications_clear")}>Clear log</button>
      </div>
      <table className="log">
        <tbody>
          {snap.notifications.length === 0 && <tr><td className="muted">Nothing yet.</td></tr>}
          {snap.notifications.map((n) => (
            <tr key={n.id} className={`log-${n.outcome}`}>
              <td className="muted small nowrap">{clock(n.at_ms)}</td>
              <td><TierMark tier={n.effective_tier} /></td>
              <td>
                {n.thread_id ? (
                  <button type="button" className="link" onClick={() => openThread(n.thread_id!)}>{n.title}</button>
                ) : (
                  n.title
                )}
                <div className="muted small">{n.body.split("\n")[0]}</div>
              </td>
              <td className="nowrap">
                <span className={`pill pill-${n.outcome}`}>{n.outcome}</span>
                {n.kind !== "new" && <span className="pill">{n.kind}</span>}
              </td>
              <td className="muted small">
                {n.reason}
                {n.banner && ` · banner${n.sound ? " + sound" : ""}${n.demand ? " + demand" : ""}`}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export type SettingsTab = "notifications" | "health" | "activity";

export function SettingsPanel({ onClose, initial = "notifications" }: { onClose: () => void; initial?: SettingsTab }) {
  const [tab, setTab] = useState<SettingsTab>(initial);
  return (
    <Modal title="Settings" onClose={onClose} wide>
      <div className="tabs">
        <button type="button" className={tab === "notifications" ? "on" : ""} onClick={() => setTab("notifications")}>Notifications</button>
        <button type="button" className={tab === "health" ? "on" : ""} onClick={() => setTab("health")}>Addresses &amp; health</button>
        <button type="button" className={tab === "activity" ? "on" : ""} onClick={() => setTab("activity")}>Notification log</button>
      </div>
      {tab === "notifications" && <Notifications />}
      {tab === "health" && <Health />}
      {tab === "activity" && <Activity />}
    </Modal>
  );
}
