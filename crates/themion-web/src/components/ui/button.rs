use leptos::prelude::*;
use leptos_ui::variants;

variants! {
    Button {
        base: "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-all disabled:pointer-events-none disabled:opacity-50 outline-none focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px] w-fit hover:cursor-pointer active:scale-[0.98]",
        variants: {
            variant: {
                Default: "bg-primary text-primary-foreground shadow-xs hover:bg-primary/90",
                Outline: "border bg-background shadow-xs hover:bg-accent hover:text-accent-foreground dark:bg-input/30 dark:border-input dark:hover:bg-input/5",
                Secondary: "bg-secondary text-secondary-foreground shadow-xs hover:bg-secondary/80",
                Ghost: "hover:bg-accent hover:text-accent-foreground dark:hover:bg-accent/50"
            },
            size: {
                Default: "h-9 px-4 py-2",
                Sm: "h-8 rounded-md px-3",
                Icon: "size-9"
            }
        },
        component: {
            element: button,
            support_href: true,
            support_aria_current: true
        }
    }
}
