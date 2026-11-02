import { useTranslation } from "react-i18next";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

function EmptyPage({
  titleKey,
  emptyKey,
}: {
  titleKey: string;
  emptyKey: string;
}) {
  const { t } = useTranslation();

  return (
    <section>
      <h1 className="text-2xl font-semibold">{t(titleKey)}</h1>
      <Card className="mt-4">
        <CardHeader>
          <CardTitle>{t(emptyKey)}</CardTitle>
        </CardHeader>
        <CardContent />
      </Card>
    </section>
  );
}

export default EmptyPage;
