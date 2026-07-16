(ns jepsen.openraft.client
  (:require [cheshire.core :as json]
            [clojure.string :as str])
  (:import (java.net URI)
           (java.net.http HttpClient
                          HttpRequest
                          HttpRequest$BodyPublishers
                          HttpResponse$BodyHandlers)
           (java.time Duration)))

(def default-api-port 21001)
(def default-raft-port 22001)

(def ^:private http-client
  (-> (HttpClient/newBuilder)
      (.connectTimeout (Duration/ofSeconds 2))
      (.build)))

(defn node-host [node]
  (if (keyword? node)
    (name node)
    (str node)))

(defn api-endpoint [test node]
  (str (node-host node) ":" (:api-port test default-api-port)))

(defn raft-addr [test node]
  (str (node-host node) ":" (:raft-port test default-raft-port)))

(defn- node-url [node path]
  (let [base (if (re-find #"^https?://" node)
               node
               (str "http://" node))]
    (str (str/replace base #"/+$" "")
         (if (str/starts-with? path "/")
           path
           (str "/" path)))))

(defn- request-builder [endpoint path]
  (doto (HttpRequest/newBuilder (URI/create (node-url endpoint path)))
    (.timeout (Duration/ofSeconds 5))))

(defn- send! [request]
  (let [response (.send http-client request (HttpResponse$BodyHandlers/ofString))
        status (.statusCode response)
        body (.body response)]
    (when-not (= 200 status)
      (throw (ex-info (str "HTTP request failed with status " status)
                      {:status status
                       :body body})))
    {:status status
     :body body}))

(defn- parse-body [response]
  (json/parse-string (:body response) true))

(defn- ok-value [response]
  (let [body (parse-body response)]
    (cond
      (contains? body :Ok) (:Ok body)
      (contains? body :Err) (throw (ex-info "OpenRaft API returned Err"
                                            {:error (:Err body)
                                             :response response}))
      :else body)))

(defn get! [endpoint path]
  (-> (request-builder endpoint path)
      (.GET)
      (.build)
      send!))

(defn post! [endpoint path body]
  (-> (request-builder endpoint path)
      (.header "Content-Type" "application/json")
      (.POST (HttpRequest$BodyPublishers/ofString (json/generate-string body)))
      (.build)
      send!))

(defn metrics! [endpoint]
  (ok-value (get! endpoint "/metrics")))

(defn init! [endpoint]
  (ok-value (post! endpoint "/init" [])))

(defn add-learner! [endpoint node-id api-addr raft-addr]
  (ok-value (post! endpoint "/add-learner"
                   {:node_id node-id
                    :api_addr api-addr
                    :raft_addr raft-addr})))

(defn change-membership! [endpoint node-ids]
  (ok-value (post! endpoint "/change-membership" node-ids)))

(defn write! [endpoint key value]
  (ok-value (post! endpoint "/write"
                   {:Set {:key key
                          :value value}})))

(defn read! [endpoint key]
  (ok-value (post! endpoint "/read" key)))
