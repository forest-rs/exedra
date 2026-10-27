declare module "*exedra_authoring_web_bridge.js" {
  import type { NativeTransport, NativeBuffers, NativeStepFile } from "./index.js";
  export default function init(): Promise<unknown>;
  export class AuthoringKernel implements NativeTransport {
    constructor();
    build_document(request: string): string;
    release_document(document: number): void;
    realize(document: number, part: number, request: string): NativeBuffers;
    export_step(document: number, part: number, request: string): NativeStepFile;
    stats_json(): string;
    workplane_json(document: number, part: number, face: number): string;
    free(): void;
  }
}
