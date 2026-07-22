(ns jepsen.openraft.client-test
  (:require [clojure.test :refer [deftest is]]
            [jepsen.openraft.client :as client]))

(defn- forward-error
  ([]
   (forward-error nil))
  ([endpoint]
   (ex-info "forward"
            {:kind :openraft-error
             :error {:ForwardToLeader
                     {:leader_id (when endpoint 2)
                      :leader_node (when endpoint
                                     {:data endpoint})}}})))

(deftest follows-leader
  (let [leader (atom "n1:21001")
        attempts (atom [])
        result (client/with-leader!
                 leader
                 ["n1:21001" "n2:21001" "n3:21001"]
                 (fn [endpoint]
                   (swap! attempts conj endpoint)
                   (if (= "n1:21001" endpoint)
                     (throw (forward-error "n2:21001"))
                     :ok)))]
    (is (= :ok result))
    (is (= ["n1:21001" "n2:21001"] @attempts))
    (is (= "n2:21001" @leader))))

(deftest searches-nodes-when-leader-is-unknown
  (let [leader (atom "n1:21001")
        attempts (atom [])
        result (client/with-leader!
                 leader
                 ["n1:21001" "n2:21001" "n3:21001"]
                 (fn [endpoint]
                   (swap! attempts conj endpoint)
                   (if (= "n3:21001" endpoint)
                     :ok
                     (throw (forward-error)))))]
    (is (= :ok result))
    (is (= ["n1:21001" "n2:21001" "n3:21001"] @attempts))
    (is (= "n3:21001" @leader))))

(deftest does-not-retry-request-timeout
  (let [leader (atom "n1:21001")
        attempts (atom 0)
        error (try
                (client/with-leader!
                  leader
                  ["n1:21001" "n2:21001" "n3:21001"]
                  (fn [_]
                    (swap! attempts inc)
                    (throw (ex-info "timeout"
                                    {:kind :request-timeout}))))
                nil
                (catch clojure.lang.ExceptionInfo e
                  e))]
    (is (= :request-timeout (:kind (ex-data error)))
        "request timeouts must reach the workload error classifier")
    (is (= 1 @attempts)
        "request timeouts must not be retried because a mutation may have committed")))
