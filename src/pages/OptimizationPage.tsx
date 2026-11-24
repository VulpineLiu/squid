import { RotateCcw, SlidersHorizontal } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import { useI18n } from "../i18n";
import { messageFrom } from "../lib/format";
import { Button, InlineError, PageHeader, Toggle, useToast } from "../components/ui";
import type { TweakState } from "../types";

export function OptimizationPage({
  embedded = false,
  onBadge,
}: {
  /** Rendered inside 系统维护 hub: page title hides, changed count flows up. */
  embedded?: boolean;
  onBadge?: (value: string | null) => void;
} = {}) {
  const { t, lang } = useI18n();
  const { push } = useToast();
  const [states, setStates] = useState<TweakState[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStates(await api.tweaksStates());
    } catch (reason) {
      setError(messageFrom(reason));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const flip = async (tweak: TweakState, next: boolean) => {
    setBusy(tweak.id);
    setError(null);
    try {
      const state = await api.tweaksSet(tweak.id, next);
      setStates((current) => current.map((row) => (row.id === state.id ? state : row)));
    } catch (reason) {
      setError(messageFrom(reason));
    } finally {
      setBusy(null);
    }
  };

  const changed = states.filter((state) => state.recorded).length;

  // Hub badge: how many tweaks have been flipped so far (null when none).
  useEffect(() => {
    if (!embedded || !onBadge) return;
    onBadge(changed > 0 ? `已更改 ${changed} 项` : null);
  }, [changed, embedded, onBadge]);

  const restoreAll = async () => {
    setBusy("__all__");
    try {
      const restored = await api.tweaksRestoreAll();
      push(
        lang === "zh"
          ? `已恢复 ${restored.length} 项为系统默认值`
          : `Restored ${restored.length} tweaks to system defaults`,
        "success",
      );
      await refresh();
    } catch (reason) {
      setError(messageFrom(reason));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className={`page optimize-page ${embedded ? "is-embedded" : ""}`}>
      {embedded ? null : (
        <PageHeader
          title={t("System optimisation", "系统优化")}
          description={t(
            "A small, curated set of reversible switches. The first write records the system's original value; one button puts every change back.",
            "少而稳的可逆开关目录。首次修改前记录系统原值，顶部一键「恢复全部默认」。每一项都先在真机验证后才会上架。",
          )}
          actions={
            <Button variant="secondary" icon={<RotateCcw size={14} />} disabled={busy !== null || changed === 0} onClick={() => void restoreAll()}>
              {t("Restore all defaults", "恢复全部默认")}
            </Button>
          }
        />
      )}
      {error ? <InlineError message={error} onDismiss={() => setError(null)} /> : null}

      <section className="optimize-group">
        <div className="optimize-group-head">
          <h2>
            <SlidersHorizontal size={15} /> {t("Everyday privacy & clarity", "常用隐私与界面")}
          </h2>
          <span className="tip">
            {changed > 0
              ? t(`${changed} changed · revertable`, `已更改 ${changed} 项 · 可随时恢复`)
              : t("Nothing changed yet", "尚未更改任何项")}
          </span>
          {/* Embedded (维护 hub): the page header — and with it the restore
              button — is hidden, so the group head carries it instead. */}
          {embedded ? (
            <Button
              variant="ghost"
              size="sm"
              icon={<RotateCcw size={13} />}
              disabled={busy !== null || changed === 0}
              onClick={() => void restoreAll()}
            >
              {t("Restore all defaults", "恢复全部默认")}
            </Button>
          ) : null}
        </div>
        <ul className="optimize-list">
          {states.map((state) => (
            <li key={state.id} className={`optimize-row ${state.enabled ? "is-on" : ""}`}>
              <div className="optimize-copy">
                <b>
                  {lang === "zh" ? state.titleZh : state.titleEn}
                  {state.restartHint && state.enabled ? (
                    <span className="optimize-restart">
                      {t("needs Explorer/Windows restart", "需重启 Explorer/系统生效")}
                    </span>
                  ) : null}
                </b>
                <small>{lang === "zh" ? state.detailZh : state.detailEn}</small>
                <small className="optimize-path">
                  {state.hive}\\{state.key} · {state.value}
                </small>
              </div>
              <span className={`optimize-risk is-${state.risk}`}>
                {state.risk === "safe" ? t("Safe", "安全") : t("Read the note", "看说明")}
              </span>
              <Toggle
                checked={state.enabled}
                disabled={busy !== null || state.unreadable}
                onChange={(next) => void flip(state, next)}
                label={lang === "zh" ? state.titleZh : state.titleEn}
              />
            </li>
          ))}
        </ul>
        {states.length === 0 && !error ? (
          <p className="optimize-empty">{t("Loading the catalog…", "正在加载目录…")}</p>
        ) : null}
      </section>
    </div>
  );
}
