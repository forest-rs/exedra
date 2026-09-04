declare module "./wasm_pkg/exedra_ops_web_bridge" {
  export default function init(): Promise<void>;
  export function list_scenarios_json(): string;
  export function run_scenario_json(name: string, optionsJson: string): string;
  export function list_inspection_scenarios_json(): string;
  export function run_inspection_scenario_json(name: string): string;
}
