declare module "./wasm_pkg/cambium_web_bridge" {
  export default function init(): Promise<void>;
  export function list_scenarios_json(): string;
  export function run_scenario_json(name: string, optionsJson: string): string;
}
