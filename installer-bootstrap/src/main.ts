import "./styles.css";
import {
  browseInstallDir,
  checkInstallDir,
  closeWizard,
  launchInstalled,
  listenProgress,
  loadContext,
  revealSetup,
  runInstall,
  type DirCheck,
  type InstallerContext,
  type InstallOutcome,
} from "./api";
import { detectLang, translate, type Lang, type MessageKey } from "./i18n";
import { nextStep, previousStep, type Step } from "./wizard";

const app = document.querySelector("#app");
if (!app) throw new Error("missing app root");

const state: {
  lang: Lang;
  step: Step;
  context: InstallerContext | null;
  dir: string;
  check: DirCheck | null;
  stage: string;
  outcome: InstallOutcome | null;
  launch: boolean;
  busy: boolean;
  error: string;
} = {
  lang: detectLang(),
  step: "welcome",
  context: null,
  dir: "",
  check: null,
  stage: "extract",
  outcome: null,
  launch: true,
  busy: false,
  error: "",
};

const root = app;

function t(key: MessageKey, vars: Record<string, string> = {}): string {
  return translate(state.lang, key, vars);
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Record<string, string> = {},
  children: Array<Node | string> = [],
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (key === "class") node.className = value;
    else node.setAttribute(key, value);
  }
  for (const child of children) {
    node.append(child);
  }
  return node;
}

function text(tag: keyof HTMLElementTagNameMap, value: string, className?: string): HTMLElement {
  const node = el(tag, className ? { class: className } : {});
  node.textContent = value;
  return node;
}

async function refreshDir(dir: string): Promise<void> {
  state.dir = dir;
  state.check = await checkInstallDir(dir);
  render();
}

function render(): void {
  const context = state.context;
  document.documentElement.lang = state.lang === "zh" ? "zh-CN" : "en";
  document.title = "TieZ";
  root.replaceChildren();

  const steps = el("ol", { class: "steps" });
  const stepKeys: Array<[Step, MessageKey]> = [
    ["welcome", "stepWelcome"],
    ["path", "stepPath"],
    ["progress", "stepProgress"],
    ["finish", "stepFinish"],
  ];
  for (const [id, key] of stepKeys) {
    const item = text("li", t(key));
    if (id === state.step) item.setAttribute("aria-current", "step");
    steps.append(item);
  }

  const langButton = el("button", { class: "lang", type: "button" });
  langButton.textContent = t("language");
  langButton.addEventListener("click", () => {
    state.lang = state.lang === "zh" ? "en" : "zh";
    render();
  });

  const card = el("section", { class: "card" });
  if (state.step === "welcome") card.append(...welcomeBody(context));
  if (state.step === "path") card.append(...pathBody());
  if (state.step === "progress") card.append(...progressBody());
  if (state.step === "finish") card.append(...finishBody());

  const version = context ? t("version", { version: context.product_version }) : "";
  root.append(
    el("main", { class: "wizard" }, [
      el("header", { class: "top" }, [
        el("div", { class: "brand" }, [
          el("img", { src: "/icon.png", alt: "" }),
          el("div", {}, [text("h1", "TieZ"), text("p", version)]),
        ]),
        langButton,
      ]),
      steps,
      card,
    ]),
  );
}

function welcomeBody(context: InstallerContext | null): Node[] {
  const actions = el("div", { class: "actions" });
  const cancel = button(t("cancel"), () => void closeWizard());
  const start = button(t("start"), () => {
    state.step = nextStep("welcome");
    render();
  }, "primary");
  actions.append(cancel, start);
  const nodes: Node[] = [
    text("h2", t("welcomeTitle")),
    text("p", t("welcomeBody"), "muted"),
    text("p", t("shortcutNote"), "note"),
    text("p", t("updateNote"), "muted"),
  ];
  if (context?.existing_version) {
    nodes.push(text("p", t("installedNote", { version: context.existing_version }), "note"));
  }
  if (context?.previous_location_differs && context.existing_location) {
    nodes.push(text("p", t("previousLocation", { location: context.existing_location }), "muted"));
  }
  nodes.push(actions);
  return nodes;
}

function pathBody(): Node[] {
  const input = el("input", { type: "text", id: "install-dir", spellcheck: "false" });
  input.value = state.dir;
  input.addEventListener("change", () => {
    void refreshDir(input.value);
  });
  const browse = button(t("browse"), () => {
    void (async () => {
      const picked = await browseInstallDir(t("pathTitle"));
      if (picked) await refreshDir(picked);
    })();
  });
  const check = state.check;
  const disk = check?.disk === "low" ? "diskLow" : check?.disk === "ok" ? "diskOk" : "diskUnknown";
  const diskClass = check?.disk === "low" ? "warn" : "muted";
  const actions = el("div", { class: "actions" });
  actions.append(
    button(t("back"), () => {
      state.step = previousStep("path");
      render();
    }),
    button(t("next"), () => void beginInstall(), "primary"),
  );
  const next = actions.querySelector(".primary");
  if (next instanceof HTMLButtonElement) next.disabled = !check?.ok || state.busy;
  const nodes: Node[] = [
    text("h2", t("pathTitle")),
    text("p", t("pathHelp"), "muted"),
    text("p", t("defaultPathNote"), "muted"),
    el("label", { class: "path", for: "install-dir" }, [
      t("pathTitle"),
      el("div", { class: "path-row" }, [input, browse]),
    ]),
  ];
  if (check?.error) nodes.push(text("p", t("invalidDir"), "bad"));
  if (check?.ok) {
    nodes.push(text("p", t("commandPreview"), "muted"));
    nodes.push(text("p", check.raw_tail, "preview"));
    nodes.push(text("p", t(disk), diskClass));
  }
  nodes.push(actions);
  return nodes;
}

function progressBody(): Node[] {
  const stage = state.stage === "install" ? "progressInstall" : "progressExtract";
  const bar = el("div", { class: "bar", role: "progressbar", "aria-valuetext": t(stage) }, [el("span")]);
  return [
    text("h2", t("progressTitle")),
    text("p", t(stage), "muted"),
    bar,
    text("p", t("progressHint"), "muted"),
  ];
}

function finishBody(): Node[] {
  const outcome = state.outcome;
  const success = outcome?.kind === "success";
  const actions = el("div", { class: "actions" });
  const nodes: Node[] = [text("h2", success ? t("finishOk") : t("finishFail"), success ? "good" : "bad")];
  if (state.error) nodes.push(text("p", state.error, "bad"));
  if (outcome && !success) {
    nodes.push(text("p", failureCopy(outcome), "muted"));
    if (outcome.exit_code >= 0) nodes.push(text("p", t("exitCode", { code: String(outcome.exit_code) }), "muted"));
    if (outcome.setup_path) nodes.push(text("p", outcome.setup_path, "preview"));
    if (outcome.raw_tail) nodes.push(text("p", outcome.raw_tail, "preview"));
    actions.append(button(t("reveal"), () => void revealSetup()));
    actions.append(button(t("retry"), () => void beginInstall(), "primary"));
  }
  if (success) {
    const label = el("label", { class: "check" });
    const box = el("input", { type: "checkbox" });
    box.checked = state.launch;
    box.addEventListener("change", () => {
      state.launch = box.checked;
    });
    label.append(box, document.createTextNode(t("launchNow")));
    nodes.push(label);
    nodes.push(text("p", t("shortcutNote"), "muted"));
    actions.append(button(t("close"), () => void finish(), "primary"));
  } else {
    actions.append(button(t("close"), () => void closeWizard()));
  }
  nodes.push(actions);
  return nodes;
}

function failureCopy(outcome: InstallOutcome): string {
  if (outcome.kind === "setup_not_embedded") return t("notEmbedded");
  if (outcome.kind === "windows_only") return t("windowsOnly");
  if (outcome.kind === "user_cancelled") return t("cancel");
  if (outcome.kind === "invalid_dir" || outcome.kind.startsWith("not_") || outcome.kind === "empty_path") return t("invalidDir");
  return t("genericFail");
}

function button(label: string, onClick: () => void, className = ""): HTMLButtonElement {
  const node = el("button", { type: "button" });
  if (className) node.className = className;
  node.textContent = label;
  node.addEventListener("click", onClick);
  return node;
}

async function beginInstall(): Promise<void> {
  const input = document.querySelector<HTMLInputElement>("#install-dir");
  if (input) state.dir = input.value;
  state.check = await checkInstallDir(state.dir);
  if (!state.check.ok || state.busy) {
    render();
    return;
  }
  state.busy = true;
  state.step = "progress";
  state.stage = "extract";
  state.error = "";
  render();
  const stop = await listenProgress((stage) => {
    state.stage = stage;
    if (state.step === "progress") render();
  });
  try {
    state.outcome = await runInstall(state.dir, (stage) => {
      state.stage = stage;
      if (state.step === "progress") render();
    });
    state.error = state.outcome.detail ?? "";
    state.step = nextStep("progress");
  } catch (error) {
    state.outcome = {
      exit_code: -1,
      kind: "failed",
      setup_path: null,
      raw_tail: state.check?.raw_tail ?? "",
      detail: null,
    };
    state.error = error instanceof Error ? error.message : String(error);
    state.step = "finish";
  } finally {
    stop();
    state.busy = false;
    render();
  }
}

async function finish(): Promise<void> {
  if (state.launch && state.outcome?.kind === "success") {
    try {
      await launchInstalled();
    } catch (error) {
      state.error = error instanceof Error ? error.message : String(error);
      render();
      return;
    }
  }
  await closeWizard();
}

async function boot(): Promise<void> {
  state.context = await loadContext();
  state.dir = state.context.default_install_dir;
  state.check = await checkInstallDir(state.dir);
  render();
}

void boot();
