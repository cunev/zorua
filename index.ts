const _req = require;
const interceptModule = _req("./intercept.node");

export const original = { x: 0, y: 0 };
export function intercept() {
  interceptModule.set_callback((X, Y, mode) => {
    //remove mode, all input received is absolute.
    original.x = X;
    original.y = Y;
  });
  interceptModule.start_input_interception();
}

export function setVirtual(position: { x: number; y: number }) {
  interceptModule.set_mouse_position(position.x, position.y);
}

export function disableThrottling() {
  interceptModule.disable_throttling();
}
