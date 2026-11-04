import { useEffect } from "react";
import { Toaster } from "sonner";
import { Film, Info, Layers, Scissors, Settings2, Wand2 } from "lucide-react";
import { inTauri, onFileDrop, pickPaths, viaBridge, VIDEO_EXTENSIONS } from "./lib/api";
import { t, useLocale } from "./lib/i18n";
import { type View, useStore } from "./lib/store";
import { ConvertView } from "./views/ConvertView";
import { CloseDialog, SettingsView, SetupView } from "./views/SettingsView";
import { InfoView, MuxView, TrimView } from "./views/Tools";

const NAV: { view: View; icon: typeof Film; key: string }[] = [
  { view: "convert", icon: Wand2, key: "nav.convert" },
  { view: "trim", icon: Scissors, key: "nav.trim" },
  { view: "mux", icon: Layers, key: "nav.mux" },
  { view: "info", icon: Info, key: "nav.info" },
];

function Nav() {
  const view = useStore((st) => st.view);
  const running = useStore((st) => st.status?.running ?? 0);
  const setView = useStore((st) => st.setView);
  const item = (v: View, Icon: typeof Film, label: string, badge?: number) => (
    <button key={v} type="button" className="nav__item" aria-current={view === v ? "page" : undefined} onClick={() => setView(v)} title={label}>
      <Icon size={19} strokeWidth={1.7} />
      <span>{label}</span>
      {badge ? <span className="nav__badge">{badge}</span> : null}
    </button>
  );
  return (
    <nav className="nav" aria-label={t("nav.label")}>
      <div className="nav__brand" aria-hidden>
        <Film size={19} />
      </div>
      {NAV.map((n) => item(n.view, n.icon, t(n.key), n.view === "convert" ? running : undefined))}
      <div className="nav__spacer" />
      {item("settings", Settings2, t("nav.settings"))}
    </nav>
  );
}

export default function App() {
  useLocale();
  const init = useStore((st) => st.init);
  const view = useStore((st) => st.view);
  const dropActive = useStore((st) => st.dropActive);
  const theme = useStore((st) => st.config?.theme ?? "system");

  useEffect(() => {
    useStore.getState().boot().catch(() => {});
  }, []);

  useEffect(() => {
    let off: (() => void) | undefined;
    onFileDrop({
      over: () => useStore.setState({ dropActive: true }),
      leave: () => useStore.setState({ dropActive: false }),
      drop: (paths) => {
        useStore.setState({ dropActive: false });
        const st = useStore.getState();
        if (st.init.kind !== "ready") return;
        if (st.dropHandler) st.dropHandler(paths);
        else {
          if (st.view !== "convert") st.setView("convert");
          st.addPaths(paths);
        }
      },
    }).then((fn) => (off = fn));
    return () => off?.();
  }, []);

  useEffect(() => {
    const onKey = async (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;
      const st = useStore.getState();
      if (e.key === "o" && st.init.kind === "ready") {
        e.preventDefault();
        const paths = await pickPaths(e.shiftKey ? { directory: true } : { multiple: true, filters: [{ name: t("common.video"), extensions: VIDEO_EXTENSIONS }] });
        st.setView("convert");
        st.addPaths(paths);
      } else if (e.key === ",") {
        e.preventDefault();
        st.setView("settings");
      } else if (e.key === "Enter" && st.view === "convert" && st.staged.some((s) => s.info)) {
        e.preventDefault();
        st.startStaged();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const ready = init.kind === "ready";
  return (
    <>
      <div className="app">
        <Nav />
        <main className="main">
          {!inTauri && <div className="mock-banner">{viaBridge ? t("app.bridgeBanner") : t("app.mockBanner")}</div>}
          {!ready && view !== "settings" ? (
            <SetupView />
          ) : view === "convert" ? (
            <ConvertView />
          ) : view === "trim" ? (
            <TrimView />
          ) : view === "mux" ? (
            <MuxView />
          ) : view === "info" ? (
            <InfoView />
          ) : (
            <SettingsView />
          )}
        </main>
        <div className="drop-overlay" data-on={dropActive} aria-hidden>
          {t("add.dropOverlay")}
        </div>
        <CloseDialog />
      </div>
      <Toaster position="bottom-right" theme={theme === "light" || theme === "dark" ? theme : "system"} closeButton richColors />
    </>
  );
}
