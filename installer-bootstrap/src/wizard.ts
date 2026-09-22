export type Step = "welcome" | "path" | "progress" | "finish";

export function nextStep(step: Step): Step {
  switch (step) {
    case "welcome":
      return "path";
    case "path":
      return "progress";
    case "progress":
      return "finish";
    case "finish":
      return "finish";
  }
}

export function previousStep(step: Step): Step {
  return step === "path" ? "welcome" : step;
}
