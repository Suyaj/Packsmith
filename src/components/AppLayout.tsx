import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import SideNav from "@/components/SideNav";

function AppLayout() {
  const { t } = useTranslation();

  return (
    <div className="flex h-screen">
      <aside className="border-r bg-sidebar">
        <header className="p-4 text-lg font-semibold">{t("app.name")}</header>
        <SideNav />
      </aside>
      <main className="flex-1 p-6">
        <Outlet />
      </main>
    </div>
  );
}

export default AppLayout;
