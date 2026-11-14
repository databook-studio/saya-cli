use super::{calendar::iso_date, rng::Lcg};

pub(crate) const CUSTOMER_COUNT: i64 = 240;
pub(crate) const ORDER_COUNT: usize = 560;

const SEED: u64 = 0x5341_5941_0000_0001;

const REGIONS: [&str; 4] = ["north", "south", "east", "west"];
const CUSTOMER_STATUSES: [&str; 4] = ["active", "active", "inactive", "churned"];
const ORDER_STATUSES: [&str; 4] = ["completed", "completed", "pending", "refunded"];
const CONTACT_CHANNELS: [&str; 3] = ["email", "phone", "slack"];

pub(crate) struct DemoData {
    pub customers: Vec<CustomerRow>,
    pub orders: Vec<OrderRow>,
    pub contacts: Vec<ContactRow>,
}

pub(crate) struct CustomerRow {
    pub id: i64,
    pub name: String,
    pub email: Option<String>,
    pub region: String,
    pub signup_date: String,
    pub status: String,
}

pub(crate) struct OrderRow {
    pub id: i64,
    pub customer_id: i64,
    pub order_date: String,
    pub amount_cents: Option<i64>,
    pub status: String,
}

pub(crate) struct ContactRow {
    pub customer_id: i64,
    pub channel: String,
    pub value: String,
}

pub(crate) fn build() -> DemoData {
    let mut lcg = Lcg::new(SEED);
    let mut customers = Vec::with_capacity(CUSTOMER_COUNT as usize);
    for id in 1..=CUSTOMER_COUNT {
        let email = if id % 9 == 0 {
            None
        } else {
            Some(format!("customer-{id:03}@example.invalid"))
        };
        let region = REGIONS[lcg.below(REGIONS.len() as u64) as usize].to_owned();
        let signup_date = iso_date(300 + lcg.below(600) as i64);
        let status = if id == 1 {
            "active"
        } else if id == 2 {
            "churned"
        } else {
            CUSTOMER_STATUSES[lcg.below(CUSTOMER_STATUSES.len() as u64) as usize]
        };
        customers.push(CustomerRow {
            id,
            name: format!("Demo Customer {id:03}"),
            email,
            region,
            signup_date,
            status: status.to_owned(),
        });
    }

    const TRAP_ORDERS: [(i64, &str); 7] = [
        (1, "2025-08-15"),
        (1, "2025-07-20"),
        (2, "2025-12-20"),
        (2, "2025-12-28"),
        (3, "2025-12-31"),
        (4, "2026-01-01"),
        (5, "2025-12-31"),
    ];
    let mut orders = Vec::with_capacity(ORDER_COUNT);
    for (index, (customer_id, order_date)) in TRAP_ORDERS.into_iter().enumerate() {
        orders.push(order_row(
            index as i64 + 1,
            customer_id,
            order_date.to_owned(),
            &mut lcg,
        ));
    }
    while orders.len() < ORDER_COUNT {
        let id = orders.len() as i64 + 1;
        let customer_id = 6 + lcg.below((CUSTOMER_COUNT - 5) as u64) as i64;
        let order_date = iso_date(lcg.below(171) as i64);
        orders.push(order_row(id, customer_id, order_date, &mut lcg));
    }

    let mut contacts = Vec::new();
    for id in 1..=CUSTOMER_COUNT {
        if (5..=24).contains(&id) {
            let rows = 2 + (id % 2);
            for seq in 0..rows {
                contacts.push(contact_row(id, seq, &mut lcg));
            }
        } else if id % 3 == 0 {
            contacts.push(contact_row(id, 0, &mut lcg));
        }
    }

    DemoData {
        customers,
        orders,
        contacts,
    }
}

fn order_row(id: i64, customer_id: i64, order_date: String, lcg: &mut Lcg) -> OrderRow {
    let amount_cents = if id % 24 == 7 {
        None
    } else {
        Some(500 + lcg.below(250_000) as i64)
    };
    let status = ORDER_STATUSES[lcg.below(ORDER_STATUSES.len() as u64) as usize].to_owned();
    OrderRow {
        id,
        customer_id,
        order_date,
        amount_cents,
        status,
    }
}

fn contact_row(customer_id: i64, seq: i64, lcg: &mut Lcg) -> ContactRow {
    let channel = CONTACT_CHANNELS[lcg.below(CONTACT_CHANNELS.len() as u64) as usize];
    let tag = format!("{customer_id:03}-{seq}");
    let value = match channel {
        "email" => format!("contact-{tag}@example.invalid"),
        "phone" => format!("+1-555-{tag}"),
        _ => format!("@demo-{tag}"),
    };
    ContactRow {
        customer_id,
        channel: channel.to_owned(),
        value,
    }
}
