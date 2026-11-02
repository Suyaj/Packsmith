import { NavLink } from "react-router-dom";
import { useTranslation } from "react-i18next";

const navItems = [
  { key: "home", to: "/" },
  { key: "projects", to: "/projects" },
  { key: "servers", to: "/servers" },
  { key: "settings", to: "/settings" },
] as const;

function SideNav() {
  const { t } = useTranslation();

  return (
    <nav aria-label="主导航" className="flex w-48 flex-col gap-1 p-4">
      {navItems.map(({ key, to }) => (
        <NavLink
          key={key}
          to={to}
          end={to === "/"}
          className={({ isActive }) =>
            `rounded-md px-3 py-2 text-sm ${
              isActive
                ? "bg-secondary text-secondary-foreground font-medium"
                : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
            }`
          }
        >
          {t(`nav.${key}`)}
        </NavLink>
      ))}
    </nav>
  );
}

export default SideNav;
