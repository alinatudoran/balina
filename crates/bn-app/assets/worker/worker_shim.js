// Classic-worker bootstrap for the bn-worker wasm module. The first message
// carries same-origin (blob) URLs of the wasm-bindgen glue (--target
// no-modules, which defines the global `wasm_bindgen`) and the wasm binary,
// plus the job request. Errors are reported back as a Failed protocol
// message. This file is embedded into bn-app via include_str!.
self.onmessage = async (ev) => {
  const { glue, wasm, request, cases } = ev.data;
  try {
    importScripts(glue);
    await wasm_bindgen({ module_or_path: wasm });
    wasm_bindgen.worker_entry(request, new Uint8Array(cases));
  } catch (e) {
    self.postMessage(JSON.stringify({ type: "Failed", error: String(e) }));
  }
};
