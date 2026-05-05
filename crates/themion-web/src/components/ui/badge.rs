use leptos::prelude::*;
use leptos_ui::variants;

variants! {
    Badge {
        base: "inline-flex items-center font-semibold rounded-md border transition-colors w-fit",
        variants: {
            variant: {
                Default: "border-transparent shadow bg-primary text-primary-foreground hover:bg-primary/80",
                Muted: "border-transparent bg-muted text-muted-foreground hover:bg-muted/80",
                Outline: "text-foreground"
            },
            size: {
                Default: "px-2.5 py-0.5 text-xs"
            }
        },
        component: {
            element: span
        }
    }
}
