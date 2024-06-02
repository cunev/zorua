const _req = require;
const interceptModule = _req("./intercept.node");

export const original = { x: 0, y: 0 };

const lastOffset = { x: 0, y: 0 };
export function intercept() {
  interceptModule.set_callback((X, Y, mode) => {
    //remove mode, all input received is absolute.
    original.x = X - lastOffset.x;
    original.y = Y - lastOffset.y;
  });
  interceptModule.start_input_interception();
}

// export function setPositionOffset(offset: { x: number; y: number }) {
//   lastOffset.x = offset.x; //To get original position in next report.
//   lastOffset.y = offset.y; //

//   interceptModule.set_mouse_position(
//     original.x + offset.x,
//     original.x + offset.y
//   );
// }

export function setPositionAbsolute(position: { x: number; y: number }) {
  lastOffset.x = position.x - original.x; //To get original position in next report.
  lastOffset.y = position.y - original.x; //

  interceptModule.set_mouse_position(position.x, position.y);
}

export function disableThrottling() {
  interceptModule.disable_throttling();
}
