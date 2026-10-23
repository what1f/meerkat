// Only sample-data changes; the design never connects to a Runtime.
import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
export function useManagementEvents(_workspace?: string) {
  "use no memo";
  const client = useQueryClient();
  useEffect(() => {
    const refresh = () => {
      void client.invalidateQueries();
    };
    addEventListener("lankide-design-data", refresh);
    return () => removeEventListener("lankide-design-data", refresh);
  }, [client]);
}
