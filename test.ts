import { mouse } from "@nut-tree/nut-js";
import { intercept } from ".";

intercept((x, y) => {
  console.log(x);
});
