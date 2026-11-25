import { RotateCcw, SlidersHorizontal } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import { useI18n } from "../i18n";
import { messageFrom } from "../lib/format";
import {
  Badge,
  Button,
  InlineError,
  PageHeader,
  Row,
  Section,
  Toggle,
  useToast,
} from "../components/ui";
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
          actions={
            <Button variant="secondary" icon={<RotateCcw size={14} />} disabled={busy !== null || changed === 0} onClick={() => void restoreAll()}>
              {t("Restore all defaults", "恢复全部默认")}
            </Button>
          }
        />
      )}
      {error ? <InlineError message={error} onDismiss={() => setError(null)} /> : null}

      <Section
        icon={<SlidersHorizontal size={16} />}
        title={t("Everyday privacy & clarity", "常用隐私与界面")}
        hint={
          changed > 0
            ? t(`${changed} changed · revertable`, `已更改 ${changed} 项 · 可随时恢复`)
            : t("Nothing changed yet", "尚未更改任何项")
        }
        actions={
          embedded ? (
            <Button
              variant="ghost"
              size="sm"
              icon={<RotateCcw size={13} />}
              disabled={busy !== null || changed === 0}
              onClick={() => void restoreAll()}
            >
              {t("Restore all defaults", "恢复全部默认")}
            </Button>
          ) : undefined
        }
      >
        <div className="row-list">
          {states.map((state) => (
            <Row
              key={state.id}
              selected={state.enabled}
              title={lang === "zh" ? state.titleZh : state.titleEn}
              subtitle={lang === "zh" ? state.detailZh : state.detailEn}
              detail={`${state.hive}\\${state.key} · ${state.value}`}
              tags={
                state.restartHint && state.enabled ? (
                  <Badge tone="warning">
                    {t("needs Explorer/Windows restart", "需重启 Explorer/系统生效")}
                  </Badge>
                ) : undefined
              }
              trailing={
                <>
                  <Badge tone={state.risk === "safe" ? "success" : "warning"}>
                    {state.risk === "safe" ? t("Safe", "安全") : t("Read the note", "看说明")}
                  </Badge>
                  <Toggle
                    checked={state.enabled}
                    disabled={busy !== null || state.unreadable}
                    onChange={(next) => void flip(state, next)}
                    label={lang === "zh" ? state.titleZh : state.titleEn}
                  />
                </>
              }
            />
          ))}
          {states.length === 0 && !error ? (
            <p className="row-empty">{t("Loading the catalog…", "正在加载目录…")}</p>
          ) : null}
        </div>
      </Section>
    </div>
  );
}
