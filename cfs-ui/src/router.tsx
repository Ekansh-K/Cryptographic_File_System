import { createBrowserRouter } from "react-router-dom";
import App from "./App";
import UnlockScreen from "./screens/UnlockScreen";
import BrowserScreen from "./screens/BrowserScreen";

export const router = createBrowserRouter([
  {
    path: "/",
    element: <App />,
    children: [
      { index: true, element: <UnlockScreen /> },
      { path: "browse/*", element: <BrowserScreen /> },
    ],
  },
]);
