use leptos::prelude::*;
use leptos_ui::clx;

mod components {
    use super::*;

    clx! {CardHeader, div, "flex flex-col items-start gap-1.5 px-6 [.border-b]:pb-6"}
    clx! {CardTitle, h2, "leading-none font-semibold"}
    clx! {CardContent, div, "px-6"}
    clx! {CardDescription, p, "text-muted-foreground text-sm"}
}

#[component]
pub fn Card(
    #[prop(into, optional)] class: String,
    children: Children,
) -> impl IntoView {
    let merged = tw_merge::tw_merge!(
        "bg-card text-card-foreground flex flex-col rounded-xl border shadow-sm py-6 gap-4",
        class
    );

    view! {
        <div class=merged data-name="Card">
            {children()}
        </div>
    }
}

pub use components::*;
