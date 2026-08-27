declare module "./wasm_pkg/cambium_web_bridge" {
  export default function init(): Promise<void>;
  export function list_scenarios_json(): string;
  export function run_scenario_json(name: string, optionsJson: string): string;
  export function list_inspection_scenarios_json(): string;
  export function run_inspection_scenario_json(name: string): string;
  export class InspectionSession {
    constructor(name: string, space: bigint);
    readonly space: bigint;
    readonly revision: bigint;
    snapshot_json(): string;
    free(): void;
  }
}
