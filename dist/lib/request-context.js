"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.requestContext = void 0;
exports.setRequestContext = setRequestContext;
exports.getRequestContext = getRequestContext;
const async_hooks_1 = require("async_hooks");
exports.requestContext = new async_hooks_1.AsyncLocalStorage();
function setRequestContext(value) {
    // enterWith keeps the store for the current async call chain
    exports.requestContext.enterWith(value);
}
function getRequestContext() {
    return exports.requestContext.getStore();
}
//# sourceMappingURL=request-context.js.map