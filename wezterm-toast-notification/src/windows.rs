#![cfg(windows)]

use crate::{ToastDuration, ToastNotification as TN};
use xml::escape::{escape_str_attribute, escape_str_pcdata};

use windows::core::{Error as WinError, IInspectable, Interface, HSTRING};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::Win32::Foundation::E_POINTER;
use windows::UI::Notifications::{
    ToastActivatedEventArgs, ToastNotification, ToastNotificationManager,
};

fn unwrap_arg<T>(a: &Option<T>) -> Result<&T, WinError> {
    match a {
        Some(t) => Ok(t),
        None => Err(WinError::new(E_POINTER, HSTRING::from("option is none"))),
    }
}

const APP_ID: &str = "org.wezfurlong.wezterm";

/// Tags are scoped to a group. Ours is specific to this process, so that
/// the same tag used by a different wezterm process refers to a different
/// notification.
fn tag_group() -> HSTRING {
    HSTRING::from(format!("wz-{}", std::process::id()))
}

/// Returns an `image` element for src, or an empty string if there is
/// no image, or if it refers to a file that doesn't exist: the system
/// may decline to show a notification that has an image that it cannot
/// load, and the text matters more than the picture.
fn image_element(src: &Option<String>, attributes: &str) -> String {
    let src = match src.as_deref() {
        Some(src) if !src.is_empty() => src,
        _ => return String::new(),
    };
    // Something like file:///C:/dir/image.png is left to the system
    // to resolve; anything else is the path of a file
    if !src.contains("://") && !std::path::Path::new(src).is_file() {
        log::warn!(
            "toast notification image {} is not a file; showing the notification without it",
            src
        );
        return String::new();
    }
    format!(
        r#"<image {}src="{}" />"#,
        attributes,
        escape_str_attribute(src)
    )
}

/// Produces the toast XML; see
/// <https://learn.microsoft.com/en-us/uwp/schemas/tiles/toastschema/schema-root>
fn toast_xml(toast: &TN) -> String {
    let url_actions = if toast.url.is_some() {
        r#"
        <actions>
           <action content="Show" arguments="show" />
        </actions>
        "#
    } else {
        ""
    };

    let duration = match toast.duration {
        ToastDuration::Short => "short",
        ToastDuration::Long => "long",
    };

    let attribution = match toast.attribution.as_ref() {
        Some(attribution) => format!(
            r#"<text placement="attribution">{}</text>"#,
            escape_str_pcdata(attribution)
        ),
        None => String::new(),
    };

    let audio = if toast.silent {
        r#"<audio silent="true" />"#
    } else {
        ""
    };

    format!(
        r#"<toast duration="{}">
        <visual>
            <binding template="ToastGeneric">
                <text>{}</text>
                <text>{}</text>
                {}
                {}
                {}
                {}
            </binding>
        </visual>
        {}
        {}
    </toast>"#,
        duration,
        escape_str_pcdata(&toast.title),
        escape_str_pcdata(&toast.message),
        attribution,
        image_element(&toast.images.icon, r#"placement="appLogoOverride" "#),
        image_element(&toast.images.inline, ""),
        image_element(&toast.images.hero, r#"placement="hero" "#),
        audio,
        url_actions
    )
}

fn show_notif_impl(toast: TN) -> Result<(), Box<dyn std::error::Error>> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(HSTRING::from(toast_xml(&toast)))?;

    let notif = ToastNotification::CreateToastNotification(xml)?;

    // toast is moved into the Activated handler below
    let tag = toast.tag.clone();
    if let Some(tag) = tag.as_ref() {
        notif.SetTag(HSTRING::from(tag.as_str()))?;
        notif.SetGroup(tag_group())?;
    }

    notif.Activated(TypedEventHandler::new(
        move |_: &Option<ToastNotification>, result: &Option<IInspectable>| {
            // let myself = unwrap_arg(myself)?;
            let result = unwrap_arg(result)?.cast::<ToastActivatedEventArgs>()?;

            let args = result.Arguments()?;

            if args == "show" {
                if let Some(url) = toast.url.as_ref() {
                    wezterm_open_url::open_url(url);
                }
            } else if args == "" {
                // We don't set the `launch` attribute, so a click on the
                // body of the notification, rather than on one of its
                // action buttons, arrives with empty arguments.
                if let Some(on_click) = toast.on_click.as_ref() {
                    on_click.call();
                }
            }

            Ok(())
        },
    ))?;

    /*
    notif.dismissed(TypedEventHandler::new(|sender, result| {
        log::info!("dismissed {:?}", result);
        Ok(())
    }))?;

    notif.failed(TypedEventHandler::new(|sender, result| {
        log::warn!("toasts are disabled {:?}", result);
        Ok(())
    }))?;
    */

    let notifier = ToastNotificationManager::CreateToastNotifierWithId(HSTRING::from(APP_ID))?;

    if let Some(tag) = tag.as_ref() {
        // Showing a notification with the same tag as one that is still on
        // screen only updates the entry in the notification center; Windows
        // doesn't show the new one on screen. Removing the earlier one first
        // makes the new one appear on screen as a fresh notification.
        if let Err(err) = dismiss_notif_impl(tag) {
            log::debug!("Failed to remove toast notification {}: {:#}", tag, err);
        }
    }

    notifier.Show(&notif)?;

    Ok(())
}

pub fn show_notif(notif: TN) -> Result<(), Box<dyn std::error::Error>> {
    // We need to be in a different thread from the caller
    // in case we get called in the guts of a windows message
    // loop dispatch and are unable to pump messages
    std::thread::spawn(move || {
        if let Err(err) = show_notif_impl(notif) {
            log::error!("Failed to show toast notification: {:#}", err);
        }
    });

    Ok(())
}

fn dismiss_notif_impl(tag: &str) -> Result<(), WinError> {
    ToastNotificationManager::History()?.RemoveGroupedTagWithId(
        HSTRING::from(tag),
        tag_group(),
        HSTRING::from(APP_ID),
    )
}

pub fn dismiss_notif(tag: &str) {
    let tag = tag.to_string();
    // Use a separate thread for the same reason as in show_notif
    std::thread::spawn(move || {
        if let Err(err) = dismiss_notif_impl(&tag) {
            log::debug!("Failed to dismiss toast notification {}: {:#}", tag, err);
        }
    });
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ToastImages;

    fn toast() -> TN {
        TN {
            title: "the title".to_string(),
            message: "the message".to_string(),
            url: None,
            timeout: None,
            duration: ToastDuration::Long,
            on_click: None,
            attribution: None,
            silent: false,
            tag: None,
            images: ToastImages::default(),
        }
    }

    /// Returns the (name, attributes) of each element, in document
    /// order, failing the test if the document is not well formed.
    fn elements(xml_text: &str) -> Vec<(String, Vec<(String, String)>)> {
        let mut result = vec![];
        for event in xml::reader::EventReader::from_str(xml_text) {
            if let xml::reader::XmlEvent::StartElement {
                name, attributes, ..
            } = event.expect("toast xml to be well formed")
            {
                result.push((
                    name.local_name,
                    attributes
                        .into_iter()
                        .map(|attr| (attr.name.local_name, attr.value))
                        .collect(),
                ));
            }
        }
        result
    }

    fn attr(name: &str, value: &str) -> (String, String) {
        (name.to_string(), value.to_string())
    }

    fn images(xml_text: &str) -> Vec<Vec<(String, String)>> {
        elements(xml_text)
            .into_iter()
            .filter(|(name, _)| name == "image")
            .map(|(_, attributes)| attributes)
            .collect()
    }

    #[test]
    fn plain() {
        let xml_text = toast_xml(&toast());
        let names: Vec<String> = elements(&xml_text).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["toast", "visual", "binding", "text", "text"]);
        assert!(xml_text.contains(r#"<toast duration="long">"#));
        assert!(xml_text.contains("<text>the title</text>"));
        assert!(xml_text.contains("<text>the message</text>"));
    }

    #[test]
    fn options() {
        let xml_text = toast_xml(&TN {
            duration: ToastDuration::Short,
            attribution: Some("from <here> & there".to_string()),
            silent: true,
            url: Some("https://example.com/".to_string()),
            ..toast()
        });
        let elements = elements(&xml_text);
        assert_eq!(
            elements[0],
            ("toast".to_string(), vec![attr("duration", "short")])
        );
        assert!(elements.contains(&("text".to_string(), vec![attr("placement", "attribution")])));
        assert!(elements.contains(&("audio".to_string(), vec![attr("silent", "true")])));
        assert!(elements.contains(&(
            "action".to_string(),
            vec![attr("content", "Show"), attr("arguments", "show")]
        )));
        assert!(xml_text.contains("from &lt;here&gt; &amp; there"));
    }

    #[test]
    fn no_images() {
        assert!(images(&toast_xml(&toast())).is_empty());
    }

    #[test]
    fn each_image_is_independent() {
        let icon = "file:///C:/images/icon.png".to_string();
        let inline = "file:///C:/images/inline.png".to_string();
        let hero = "file:///C:/images/a \"b\" & <c>.png".to_string();

        let xml_text = toast_xml(&TN {
            images: ToastImages {
                icon: Some(icon.clone()),
                inline: None,
                hero: None,
            },
            ..toast()
        });
        assert_eq!(
            images(&xml_text),
            vec![vec![
                attr("placement", "appLogoOverride"),
                attr("src", &icon)
            ]]
        );

        let xml_text = toast_xml(&TN {
            images: ToastImages {
                icon: None,
                inline: Some(inline.clone()),
                hero: None,
            },
            ..toast()
        });
        assert_eq!(images(&xml_text), vec![vec![attr("src", &inline)]]);

        // The attribute value must survive being escaped
        let xml_text = toast_xml(&TN {
            images: ToastImages {
                icon: Some(icon.clone()),
                inline: Some(inline.clone()),
                hero: Some(hero.clone()),
            },
            ..toast()
        });
        assert_eq!(
            images(&xml_text),
            vec![
                vec![attr("placement", "appLogoOverride"), attr("src", &icon)],
                vec![attr("src", &inline)],
                vec![attr("placement", "hero"), attr("src", &hero)],
            ]
        );
    }

    #[test]
    fn missing_image_file_is_left_out() {
        let xml_text = toast_xml(&TN {
            images: ToastImages {
                icon: Some("C:\\this\\file\\does\\not\\exist.png".to_string()),
                inline: Some(String::new()),
                hero: None,
            },
            ..toast()
        });
        assert!(images(&xml_text).is_empty());
    }

    #[test]
    fn existing_image_file_is_used() {
        // Any file will do; the system is what decides whether it is an image
        let path = std::env::current_exe().unwrap();
        let path = path.to_str().unwrap().to_string();
        let xml_text = toast_xml(&TN {
            images: ToastImages {
                icon: None,
                inline: None,
                hero: Some(path.clone()),
            },
            ..toast()
        });
        assert_eq!(
            images(&xml_text),
            vec![vec![attr("placement", "hero"), attr("src", &path)]]
        );
    }
}
